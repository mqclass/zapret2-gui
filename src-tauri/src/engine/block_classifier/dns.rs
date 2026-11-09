//! Minimal DNS wire-format codec + resolvers for the block classifier.
//!
//! One codec serves both transports the classifier's reference resolution
//! uses (§2 of the design): DoH (RFC 8484, the wire message is the POST
//! body) and plain UDP/53 (the non-evidence fallback). Only what the
//! classifier needs exists here: A/AAAA questions, ANSWER-section parsing,
//! CNAME chasing to depth 2. Nothing else in the app speaks DNS wire
//! format, and this module deliberately does not grow into a general
//! resolver: no TCP/53 fallback (a truncated UDP answer is a channel
//! failure, §2.3), no authority/additional parsing beyond skipping.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::net::UdpSocket;
use tokio::time::timeout;

pub const QTYPE_A: u16 = 1;
pub const QTYPE_AAAA: u16 = 28;
const QTYPE_CNAME: u16 = 5;
const QCLASS_IN: u16 = 1;

/// Errors the codec layer can report. Transports map these to a plain
/// channel failure: for classification, *why* a reference channel failed
/// matters less than the fact that it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsCodecError {
    TooShort,
    IdMismatch,
    NotResponse,
    /// RCODE other than NOERROR/NXDOMAIN (e.g. SERVFAIL = 2).
    ServerFailure(u8),
    /// TC bit set: the answer does not fit one datagram. Per design this
    /// is a channel failure, never a partial answer.
    Truncated,
    Malformed(&'static str),
}

impl std::fmt::Display for DnsCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsCodecError::TooShort => write!(f, "dns message too short"),
            DnsCodecError::IdMismatch => write!(f, "dns response id mismatch"),
            DnsCodecError::NotResponse => write!(f, "dns message is not a response"),
            DnsCodecError::ServerFailure(rc) => write!(f, "dns server failure, rcode {rc}"),
            DnsCodecError::Truncated => write!(f, "dns response truncated (TC)"),
            DnsCodecError::Malformed(what) => write!(f, "dns message malformed: {what}"),
        }
    }
}

/// Parsed ANSWER section of one response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DnsAnswer {
    pub addrs: Vec<IpAddr>,
    /// First CNAME target seen, if any (used for chasing when the same
    /// response carries no address records for the target).
    pub cname: Option<String>,
}

/// Encodes one question (RD=1, one question, QCLASS=IN).
/// Encodes a one-question DNS query. Returns `None` for a name that
/// cannot be represented on the wire: a label over 63 bytes (RFC 1035
/// §2.3.4) has no valid encoding, and silently truncating it would ask
/// the resolver about a *different* name than the one intended — the
/// caller must fail the resolution instead.
pub fn encode_query(id: u16, name: &str, qtype: u16) -> Option<Vec<u8>> {
    let mut msg = Vec::with_capacity(12 + name.len() + 6);
    msg.extend_from_slice(&id.to_be_bytes());
    msg.extend_from_slice(&0x0100u16.to_be_bytes()); // RD=1
    msg.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    msg.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    msg.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    msg.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    if !encode_name(&mut msg, name) {
        return None;
    }
    msg.extend_from_slice(&qtype.to_be_bytes());
    msg.extend_from_slice(&QCLASS_IN.to_be_bytes());
    Some(msg)
}

/// Appends the wire form of `name`; `false` when a label exceeds the
/// 63-byte wire limit and the name cannot be encoded as-is.
fn encode_name(msg: &mut Vec<u8>, name: &str) -> bool {
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        let bytes = label.as_bytes();
        if bytes.len() > 63 {
            return false;
        }
        msg.push(bytes.len() as u8);
        msg.extend_from_slice(bytes);
    }
    msg.push(0);
    true
}

/// Reads a possibly-compressed name at `offset`; returns the name and the
/// offset just past the name *at its original location* (past the first
/// pointer when compression is used).
fn read_name(msg: &[u8], offset: usize) -> Result<(String, usize), DnsCodecError> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = offset;
    let mut end: Option<usize> = None;
    let mut jumps = 0usize;
    loop {
        if pos >= msg.len() {
            return Err(DnsCodecError::Malformed("name out of bounds"));
        }
        let len = msg[pos];
        if len == 0 {
            if end.is_none() {
                end = Some(pos + 1);
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= msg.len() {
                return Err(DnsCodecError::Malformed("truncated compression pointer"));
            }
            let ptr = (((len as usize) & 0x3F) << 8) | msg[pos + 1] as usize;
            if end.is_none() {
                end = Some(pos + 2);
            }
            jumps += 1;
            if jumps > 16 {
                return Err(DnsCodecError::Malformed("compression pointer loop"));
            }
            pos = ptr;
            continue;
        }
        if len & 0xC0 != 0 {
            return Err(DnsCodecError::Malformed("reserved label bits"));
        }
        let start = pos + 1;
        let stop = start + len as usize;
        if stop > msg.len() {
            return Err(DnsCodecError::Malformed("label out of bounds"));
        }
        labels.push(String::from_utf8_lossy(&msg[start..stop]).into_owned());
        pos = stop;
    }
    Ok((labels.join("."), end.unwrap_or(pos + 1)))
}

fn skip_name(msg: &[u8], offset: usize) -> Result<usize, DnsCodecError> {
    Ok(read_name(msg, offset)?.1)
}

/// Parses one response message. `expect_id`, when set, must match the
/// header ID (the UDP transport checks this; DoH sends ID 0 per RFC 8484
/// §4 and checks the echo the same way).
pub fn parse_response(msg: &[u8], expect_id: Option<u16>) -> Result<DnsAnswer, DnsCodecError> {
    if msg.len() < 12 {
        return Err(DnsCodecError::TooShort);
    }
    let id = u16::from_be_bytes([msg[0], msg[1]]);
    if let Some(want) = expect_id {
        if id != want {
            return Err(DnsCodecError::IdMismatch);
        }
    }
    let flags = u16::from_be_bytes([msg[2], msg[3]]);
    if flags & 0x8000 == 0 {
        return Err(DnsCodecError::NotResponse);
    }
    if flags & 0x0200 != 0 {
        return Err(DnsCodecError::Truncated);
    }
    let rcode = (flags & 0x000F) as u8;
    // NXDOMAIN (3) is a legitimate "no such name" answer with an empty
    // ANSWER section, not a channel failure.
    if rcode != 0 && rcode != 3 {
        return Err(DnsCodecError::ServerFailure(rcode));
    }
    let qd = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let an = u16::from_be_bytes([msg[6], msg[7]]) as usize;

    let mut pos = 12usize;
    for _ in 0..qd {
        pos = skip_name(msg, pos)?;
        if pos + 4 > msg.len() {
            return Err(DnsCodecError::TooShort);
        }
        pos += 4;
    }

    let mut out = DnsAnswer::default();
    for _ in 0..an {
        pos = skip_name(msg, pos)?;
        if pos + 10 > msg.len() {
            return Err(DnsCodecError::TooShort);
        }
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let rdlen = u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]) as usize;
        let rdata = pos + 10;
        if rdata + rdlen > msg.len() {
            return Err(DnsCodecError::TooShort);
        }
        match rtype {
            QTYPE_A if rdlen == 4 => {
                out.addrs.push(IpAddr::V4(Ipv4Addr::new(
                    msg[rdata],
                    msg[rdata + 1],
                    msg[rdata + 2],
                    msg[rdata + 3],
                )));
            }
            QTYPE_AAAA if rdlen == 16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&msg[rdata..rdata + 16]);
                out.addrs.push(IpAddr::V6(Ipv6Addr::from(octets)));
            }
            QTYPE_CNAME => {
                if out.cname.is_none() {
                    let (target, _) = read_name(msg, rdata)?;
                    out.cname = Some(target);
                }
            }
            _ => {}
        }
        pos = rdata + rdlen;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_query_id() -> u16 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    (nanos ^ (n << 7) ^ (n >> 3)) as u16
}

/// One UDP query with one retry on timeout (design §2.3). ID and QR are
/// verified by the codec parse.
async fn udp_query(
    server: IpAddr,
    name: &str,
    qtype: u16,
    per_try: Duration,
) -> Result<DnsAnswer, String> {
    let bind: IpAddr = match server {
        IpAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
        IpAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
    };
    let sock = UdpSocket::bind((bind, 0)).await.map_err(|e| e.to_string())?;
    sock.connect((server, 53)).await.map_err(|e| e.to_string())?;
    let mut last_err = String::new();
    for _ in 0..2 {
        let id = next_query_id();
        let query = encode_query(id, name, qtype)
            .ok_or_else(|| "invalid dns name".to_string())?;
        sock.send(&query).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 2048];
        match timeout(per_try, sock.recv(&mut buf)).await {
            Ok(Ok(n)) => match parse_response(&buf[..n], Some(id)) {
                Ok(ans) => return Ok(ans),
                Err(e) => return Err(e.to_string()),
            },
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                last_err = "udp dns timeout".to_string();
            }
        }
    }
    Err(last_err)
}

/// Resolves `name` over plain UDP against `server`, A and AAAA as
/// separate messages, chasing CNAME to depth 2 (§2.3). The result is NOT
/// evidence-grade (§2.2); callers must treat it accordingly. Addresses are
/// returned unfiltered — the caller's `is_public_ip` gate (§3.3) applies
/// to every address before any use.
pub async fn resolve_via_udp(
    server: IpAddr,
    name: &str,
    per_try: Duration,
) -> Result<Vec<IpAddr>, String> {
    let mut all: Vec<IpAddr> = Vec::new();
    let mut current = name.to_string();
    for _depth in 0..=2 {
        let a = udp_query(server, &current, QTYPE_A, per_try).await?;
        let aaaa = udp_query(server, &current, QTYPE_AAAA, per_try).await?;
        let mut cname = a.cname.clone().or(aaaa.cname.clone());
        for ans in [&a, &aaaa] {
            for ip in &ans.addrs {
                if !all.contains(ip) {
                    all.push(*ip);
                }
            }
        }
        if !all.is_empty() {
            return Ok(all);
        }
        match cname.take() {
            Some(next) => current = next,
            None => return Ok(all),
        }
    }
    Ok(all)
}

/// One DoH query (POST, `application/dns-message`, wire ID 0 per RFC 8484
/// §4: the TLS channel is the authentication, the ID only helps caches).
async fn doh_query(
    client: &reqwest::Client,
    url: &str,
    name: &str,
    qtype: u16,
) -> Result<DnsAnswer, String> {
    let query = encode_query(0, name, qtype)
        .ok_or_else(|| "invalid dns name".to_string())?;
    let resp = client
        .post(url)
        .header("content-type", "application/dns-message")
        .header("accept", "application/dns-message")
        .body(query)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("doh http {}", resp.status().as_u16()));
    }
    // Body cap: a DNS message fits in 64 KiB by construction, so read
    // chunked and refuse anything larger instead of buffering an
    // unbounded body from the network into the admin process.
    const MAX_DOH_BODY: usize = 64 * 1024;
    let mut resp = resp;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_DOH_BODY {
            return Err("doh response too large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_response(&bytes, Some(0)).map_err(|e| e.to_string())
}

/// Resolves `name` over DoH with the given (caller-pinned) client,
/// chasing CNAME to depth 2. Same contract as [`resolve_via_udp`] minus
/// the evidence caveat: this channel IS evidence-grade (§2.1).
pub async fn resolve_via_doh(
    client: &reqwest::Client,
    url: &str,
    name: &str,
) -> Result<Vec<IpAddr>, String> {
    let mut all: Vec<IpAddr> = Vec::new();
    let mut current = name.to_string();
    for _depth in 0..=2 {
        let a = doh_query(client, url, &current, QTYPE_A).await?;
        let aaaa = doh_query(client, url, &current, QTYPE_AAAA).await?;
        let mut cname = a.cname.clone().or(aaaa.cname.clone());
        for ans in [&a, &aaaa] {
            for ip in &ans.addrs {
                if !all.contains(ip) {
                    all.push(*ip);
                }
            }
        }
        if !all.is_empty() {
            return Ok(all);
        }
        match cname.take() {
            Some(next) => current = next,
            None => return Ok(all),
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny response builder producing wire bytes the way a real resolver
    /// emits them (compression pointer to the question name, SOA in
    /// authority for NODATA). Fixtures mirror recorded resolver answers
    /// structurally: live capture is impossible from the build sandbox
    /// (UDP/53 egress blocked), so the shapes are reproduced byte-for-byte
    /// per RFC 1035 instead.
    struct Resp {
        buf: Vec<u8>,
    }

    impl Resp {
        fn new(id: u16, flags: u16, name: &str, qtype: u16) -> Self {
            let mut buf = Vec::new();
            buf.extend_from_slice(&id.to_be_bytes());
            buf.extend_from_slice(&flags.to_be_bytes());
            buf.extend_from_slice(&1u16.to_be_bytes()); // QD
            buf.extend_from_slice(&0u16.to_be_bytes()); // AN (patched later)
            buf.extend_from_slice(&0u16.to_be_bytes()); // NS (patched later)
            buf.extend_from_slice(&0u16.to_be_bytes()); // AR
            encode_name(&mut buf, name);
            buf.extend_from_slice(&qtype.to_be_bytes());
            buf.extend_from_slice(&1u16.to_be_bytes());
            Resp { buf }
        }
        fn answer_a(mut self, ip: [u8; 4]) -> Self {
            self.buf[6] = 0;
            self.buf[7] += 1;
            self.buf.extend_from_slice(&[0xC0, 0x0C]); // pointer to question name
            self.buf.extend_from_slice(&QTYPE_A.to_be_bytes());
            self.buf.extend_from_slice(&1u16.to_be_bytes());
            self.buf.extend_from_slice(&300u32.to_be_bytes());
            self.buf.extend_from_slice(&4u16.to_be_bytes());
            self.buf.extend_from_slice(&ip);
            self
        }
        fn answer_aaaa(mut self, ip: [u8; 16]) -> Self {
            self.buf[7] += 1;
            self.buf.extend_from_slice(&[0xC0, 0x0C]);
            self.buf.extend_from_slice(&QTYPE_AAAA.to_be_bytes());
            self.buf.extend_from_slice(&1u16.to_be_bytes());
            self.buf.extend_from_slice(&300u32.to_be_bytes());
            self.buf.extend_from_slice(&16u16.to_be_bytes());
            self.buf.extend_from_slice(&ip);
            self
        }
        fn answer_cname(mut self, target: &str) -> Self {
            self.buf[7] += 1;
            self.buf.extend_from_slice(&[0xC0, 0x0C]);
            self.buf.extend_from_slice(&QTYPE_CNAME.to_be_bytes());
            self.buf.extend_from_slice(&1u16.to_be_bytes());
            self.buf.extend_from_slice(&300u32.to_be_bytes());
            let mut rd = Vec::new();
            encode_name(&mut rd, target);
            self.buf.extend_from_slice(&(rd.len() as u16).to_be_bytes());
            self.buf.extend_from_slice(&rd);
            self
        }
        /// A record whose owner name is a literal (uncompressed) name —
        /// used after a CNAME to give the target its address inline.
        fn answer_a_named(mut self, name: &str, ip: [u8; 4]) -> Self {
            self.buf[7] += 1;
            encode_name(&mut self.buf, name);
            self.buf.extend_from_slice(&QTYPE_A.to_be_bytes());
            self.buf.extend_from_slice(&1u16.to_be_bytes());
            self.buf.extend_from_slice(&300u32.to_be_bytes());
            self.buf.extend_from_slice(&4u16.to_be_bytes());
            self.buf.extend_from_slice(&ip);
            self
        }
        fn authority_soa(mut self) -> Self {
            self.buf[9] += 1; // NSCOUNT
            self.buf.extend_from_slice(&[0xC0, 0x0C]);
            self.buf.extend_from_slice(&6u16.to_be_bytes()); // SOA
            self.buf.extend_from_slice(&1u16.to_be_bytes());
            self.buf.extend_from_slice(&60u32.to_be_bytes());
            // Minimal SOA rdata (20 bytes fixed part only; parser skips by rdlen)
            self.buf.extend_from_slice(&20u16.to_be_bytes());
            self.buf.extend_from_slice(&[0u8; 20]);
            self
        }
        fn bytes(self) -> Vec<u8> {
            self.buf
        }
    }

    const QR_RD_RA: u16 = 0x8180; // QR + RD + RA, NOERROR

    #[test]
    fn query_encoding_shape() {
        let q = encode_query(0x1234, "www.example.com", QTYPE_A).unwrap();
        assert_eq!(&q[0..2], &[0x12, 0x34]);
        assert_eq!(&q[2..4], &[0x01, 0x00]); // RD
        assert_eq!(&q[4..6], &[0x00, 0x01]); // QD=1
        assert_eq!(q[12], 3); // "www"
        let ans = parse_response(&q, Some(0x1234));
        assert!(matches!(ans, Err(DnsCodecError::NotResponse)));
    }

    #[test]
    fn encode_query_rejects_overlong_label() {
        // A label over the 63-byte wire limit must fail the encode,
        // never silently truncate into a different name.
        let long_label = "a".repeat(64);
        let name = format!("{}.example.com", long_label);
        assert!(encode_query(1, &name, QTYPE_A).is_none());
        let max_label = "a".repeat(63);
        let name = format!("{}.example.com", max_label);
        assert!(encode_query(1, &name, QTYPE_A).is_some());
    }

    #[test]
    fn parses_cname_chain_answer() {
        // www.example.com -> CNAME edge.example.net -> A 203.0.113.7 in ONE response.
        // (203.0.113.0/24 is TEST-NET-3 in the fixture only; parsing must not filter —
        // filtering is the caller's is_public_ip gate.)
        let msg = Resp::new(0x2222, QR_RD_RA, "www.example.com", QTYPE_A)
            .answer_cname("edge.example.net")
            .answer_a_named("edge.example.net", [203, 0, 113, 7])
            .bytes();
        let ans = parse_response(&msg, Some(0x2222)).unwrap();
        assert_eq!(ans.cname.as_deref(), Some("edge.example.net"));
        assert_eq!(ans.addrs, vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))]);
    }

    #[test]
    fn parses_aaaa_only_answer() {
        let mut v6 = [0u8; 16];
        v6[0] = 0x26;
        v6[1] = 0x06;
        v6[15] = 0x01;
        let msg = Resp::new(0x3333, QR_RD_RA, "v6.example.net", QTYPE_AAAA)
            .answer_aaaa(v6)
            .bytes();
        let ans = parse_response(&msg, Some(0x3333)).unwrap();
        assert_eq!(ans.addrs, vec![IpAddr::V6(Ipv6Addr::from(v6))]);
        assert!(ans.cname.is_none());
    }

    #[test]
    fn nodata_response_is_empty_not_error() {
        let msg = Resp::new(0x4444, QR_RD_RA, "empty.example.net", QTYPE_A)
            .authority_soa()
            .bytes();
        let ans = parse_response(&msg, Some(0x4444)).unwrap();
        assert!(ans.addrs.is_empty());
        assert!(ans.cname.is_none());
    }

    #[test]
    fn servfail_is_channel_error() {
        let msg = Resp::new(0x5555, 0x8182, "x.example.net", QTYPE_A).bytes(); // RCODE=2
        assert_eq!(
            parse_response(&msg, Some(0x5555)),
            Err(DnsCodecError::ServerFailure(2))
        );
    }

    #[test]
    fn truncated_flag_is_channel_error() {
        let msg = Resp::new(0x6666, QR_RD_RA | 0x0200, "x.example.net", QTYPE_A).bytes();
        assert_eq!(
            parse_response(&msg, Some(0x6666)),
            Err(DnsCodecError::Truncated)
        );
    }

    #[test]
    fn wrong_id_rejected() {
        let msg = Resp::new(0x7777, QR_RD_RA, "x.example.net", QTYPE_A)
            .answer_a([192, 0, 2, 1])
            .bytes();
        assert_eq!(
            parse_response(&msg, Some(0x7778)),
            Err(DnsCodecError::IdMismatch)
        );
    }

    #[test]
    fn nxdomain_is_empty_answer() {
        let msg = Resp::new(0x1111, 0x8183, "gone.example.net", QTYPE_A).bytes(); // RCODE=3
        let ans = parse_response(&msg, Some(0x1111)).unwrap();
        assert!(ans.addrs.is_empty());
    }
}
