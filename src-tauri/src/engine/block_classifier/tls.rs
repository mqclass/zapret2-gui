//! Manual tokio-rustls connector for the classifier's TLS legs.
//!
//! Step 0 decision (BLOCK-CLASSIFIER-STEP0-DECISION.md): reqwest cannot
//! expose handshake stages — an RST after ClientHello and an RST after
//! the Certificate flight are byte-identical through its API — so the
//! taxonomy of design §1.2 is measured here instead. The mechanism is
//! an *observing* certificate verifier: it delegates the real verdict
//! to WebPkiServerVerifier, records (stage reached, verdict, subject)
//! for the classifier, and then lets the handshake continue regardless
//! — whether the connection lives or dies after the certificate stage
//! is itself the signal (`PostCertKilled` vs `CertInvalid` vs `Http`).
//!
//! The foreign leg uses a separate accept-anything verifier and never
//! reads a body: its only product is the fact of handshake survival
//! (design §3.3 item 5).

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::{ForeignOutcome, TlsRealOutcome};

/// What the observing verifier saw. `valid` is the WebPki verdict for
/// the requested name: `Some(true)`/`Some(false)` once the certificate
/// stage was reached, `None` before it (or for the foreign leg, which
/// does not judge).
#[derive(Debug, Clone, Default)]
pub struct CertObservation {
    pub stage_reached: bool,
    pub valid: Option<bool>,
    /// SAN names of the presented certificate, one line (log use only,
    /// design §3.3 item 7).
    pub subject: Option<String>,
}

type SharedObs = Arc<Mutex<CertObservation>>;

fn snapshot(obs: &SharedObs) -> CertObservation {
    obs.lock().map(|o| o.clone()).unwrap_or_default()
}

/// Names from the certificate's subjectAltName extension, extracted
/// with a minimal DER scan (rustls-webpki keeps its name iterator
/// crate-private). Log decoration only — never a decision input.
fn subject_line(cert: &CertificateDer<'_>) -> Option<String> {
    let names = cert_san_names(cert.as_ref());
    if names.is_empty() {
        None
    } else {
        Some(names.join(","))
    }
}

fn cert_san_names(der: &[u8]) -> Vec<String> {
    // OID 2.5.29.17 (subjectAltName) DER header: 06 03 55 1D 11.
    const SAN_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x1D, 0x11];
    let pos = match der
        .windows(SAN_OID.len())
        .position(|w| w == SAN_OID)
        .map(|p| p + SAN_OID.len())
    {
        Some(p) => p,
        None => return Vec::new(),
    };
    let mut rest = &der[pos..];
    // Optional critical BOOLEAN, then the OCTET STRING value.
    if rest.first() == Some(&0x01) {
        rest = tlv_tail(rest);
    }
    if rest.first() != Some(&0x04) {
        return Vec::new();
    }
    let value = match read_tlv(rest) {
        Some((_, contents)) => contents,
        None => return Vec::new(),
    };
    // value = SEQUENCE OF GeneralName; dNSName is [2] IA5String,
    // iPAddress is [7] OCTET STRING.
    let seq = match read_tlv(value) {
        Some((0x30, contents)) => contents,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut cur = seq;
    while !cur.is_empty() {
        let (tag, entry) = match read_tlv(cur) {
            Some(v) => v,
            None => break,
        };
        match tag {
            0x82 => {
                if let Ok(s) = std::str::from_utf8(entry) {
                    out.push(s.to_string());
                }
            }
            0x87 => {
                if entry.len() == 4 {
                    out.push(IpAddr::from([entry[0], entry[1], entry[2], entry[3]]).to_string());
                } else if entry.len() == 16 {
                    let mut b = [0u8; 16];
                    b.copy_from_slice(entry);
                    out.push(IpAddr::from(b).to_string());
                }
            }
            _ => {}
        }
        cur = tlv_tail(cur);
    }
    out
}

/// Splits `input` into (tag, contents) of its first TLV. Lengths are
/// assumed well-formed (we only feed it real certificates).
fn read_tlv(input: &[u8]) -> Option<(u8, &[u8])> {
    let (len, hdr) = tlv_len(input)?;
    if input.len() < hdr + len {
        return None;
    }
    Some((input[0], &input[hdr..hdr + len]))
}

/// The slice after the first TLV in `input`.
fn tlv_tail(input: &[u8]) -> &[u8] {
    match tlv_len(input) {
        Some((len, hdr)) if input.len() >= hdr + len => &input[hdr + len..],
        _ => &[],
    }
}

/// (content length, header length) of the first TLV in `input`.
fn tlv_len(input: &[u8]) -> Option<(usize, usize)> {
    if input.len() < 2 {
        return None;
    }
    let b = input[1];
    if b & 0x80 == 0 {
        Some((b as usize, 2))
    } else {
        let n = (b & 0x7F) as usize;
        if n == 0 || n > 4 || input.len() < 2 + n {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | input[2 + i] as usize;
        }
        Some((len, 2 + n))
    }
}

/// Verifier for the real leg: judges via WebPki, records, never aborts
/// on the verdict itself. Signature verification still delegates to
/// WebPki, so a copied certificate without the private key fails the
/// handshake after the certificate stage (→ PostCertKilled), exactly
/// like a real client would experience.
struct ObservingVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    obs: SharedObs,
}

impl std::fmt::Debug for ObservingVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservingVerifier").finish_non_exhaustive()
    }
}

impl ServerCertVerifier for ObservingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let verdict = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        if let Ok(mut o) = self.obs.lock() {
            o.stage_reached = true;
            o.valid = Some(verdict.is_ok());
            o.subject = subject_line(end_entity);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Verifier for the foreign leg: accepts everything, records only that
/// the certificate stage was reached. The response is never used as
/// data (design §3.3 item 5).
struct AcceptingVerifier {
    obs: SharedObs,
}

impl std::fmt::Debug for AcceptingVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptingVerifier").finish_non_exhaustive()
    }
}

impl ServerCertVerifier for AcceptingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        if let Ok(mut o) = self.obs.lock() {
            o.stage_reached = true;
            o.subject = subject_line(end_entity);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

/// WebPki roots, built once. The crypto provider is pinned to ring —
/// the provider the project tree already uses — so config building
/// never depends on feature-selection order.
pub fn webpki_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let mut store = RootCertStore::empty();
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(store)
        })
        .clone()
}

fn real_config(
    roots: Arc<RootCertStore>,
    obs: SharedObs,
) -> Result<Arc<ClientConfig>, RustlsError> {
    let webpki = WebPkiServerVerifier::builder(roots)
        .build()
        .map_err(|e| RustlsError::General(e.to_string()))?;
    let mut cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ObservingVerifier {
            inner: webpki,
            obs,
        }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

fn foreign_config(obs: SharedObs) -> Arc<ClientConfig> {
    let mut cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptingVerifier { obs }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(cfg)
}

fn server_name(name: &str) -> Result<ServerName<'static>, RustlsError> {
    if let Ok(ip) = name.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    Ok(ServerName::DnsName(
        rustls::pki_types::DnsName::try_from(name.to_string())
            .map_err(|_| RustlsError::General("invalid server name".to_string()))?,
    ))
}

/// Fate of one TLS attempt. A failed TCP connect *inside* the attempt
/// folds into `KilledPreCert`: the connection died before the
/// certificate stage, which is literally what the outcome states.
/// TCP liveness as evidence is carried by the dedicated TCP rounds,
/// never inferred from TLS attempts (see `AddrSignals`).
pub enum AttemptFate {
    Real(TlsRealOutcome),
    Foreign(ForeignOutcome),
}

pub struct AttemptResult {
    pub fate: AttemptFate,
    pub cert: CertObservation,
}

/// One `tls_real` attempt: TCP to the pinned `ip`, TLS with the real
/// name, then — only over a valid session — a hand-written HTTP/1.1
/// `GET /` of which only the status line is read (≤64 KB, design §3.2).
pub async fn tls_real_attempt(
    ip: IpAddr,
    port: u16,
    name: &str,
    roots: Arc<RootCertStore>,
    budget: Duration,
) -> AttemptResult {
    let obs: SharedObs = Arc::new(Mutex::new(CertObservation::default()));
    let fate = match tokio::time::timeout(
        budget,
        real_inner(ip, port, name, roots, obs.clone()),
    )
    .await
    {
        Ok(f) => f,
        Err(_) => {
            // Budget exhausted: a timeout is Timeout at ANY stage
            // (taxonomy §1.2). Death was never observed — only the
            // absence of progress — so a timeout after the cert stage
            // must not be dressed up as PostCertKilled. What the
            // verifier did see stays available in the returned cert
            // observation; it does not reclassify the outcome.
            // (Contrast real_inner's error arm: an actual connection
            // error after the cert stage IS PostCertKilled.)
            AttemptFate::Real(TlsRealOutcome::Timeout)
        }
    };
    AttemptResult {
        cert: snapshot(&obs),
        fate,
    }
}

async fn real_inner(
    ip: IpAddr,
    port: u16,
    name: &str,
    roots: Arc<RootCertStore>,
    obs: SharedObs,
) -> AttemptFate {
    let tcp = match TcpStream::connect(SocketAddr::new(ip, port)).await {
        Ok(s) => s,
        Err(_) => return AttemptFate::Real(TlsRealOutcome::KilledPreCert),
    };
    let sni = match server_name(name) {
        Ok(n) => n,
        Err(_) => return AttemptFate::Real(TlsRealOutcome::KilledPreCert),
    };
    let config = match real_config(roots, obs.clone()) {
        Ok(c) => c,
        Err(e) => {
            // Building a verifier over the static roots cannot fail
            // in practice; if it ever does, the attempt died before
            // any network event — logged, and reported as such
            // (KilledPreCert), never a panic in the admin process.
            eprintln!("[classifier] failed to build TLS verifier: {}", e);
            return AttemptFate::Real(TlsRealOutcome::KilledPreCert);
        }
    };
    let connector = TlsConnector::from(config);
    match connector.connect(sni, tcp).await {
        Ok(mut stream) => {
            let o = snapshot(&obs);
            if o.valid == Some(false) {
                // The certificate is not the target's: whatever answers
                // further is a stub/MITM, never an Http outcome.
                return AttemptFate::Real(TlsRealOutcome::CertInvalid);
            }
            let host_header = if name.parse::<IpAddr>().is_ok() {
                ip.to_string()
            } else {
                name.to_string()
            };
            let req = format!(
                "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: zapret2-classifier\r\nAccept: */*\r\n\r\n",
                host_header
            );
            if stream.write_all(req.as_bytes()).await.is_err() {
                return AttemptFate::Real(TlsRealOutcome::PostCertKilled);
            }
            match read_status_line(&mut stream).await {
                Some(code) => AttemptFate::Real(TlsRealOutcome::Http(code)),
                None => AttemptFate::Real(TlsRealOutcome::PostCertKilled),
            }
        }
        Err(_) => {
            let o = snapshot(&obs);
            if o.stage_reached {
                match o.valid {
                    Some(false) => AttemptFate::Real(TlsRealOutcome::CertInvalid),
                    _ => AttemptFate::Real(TlsRealOutcome::PostCertKilled),
                }
            } else {
                AttemptFate::Real(TlsRealOutcome::KilledPreCert)
            }
        }
    }
}

/// One `tls_foreign` attempt: same pinned IP, neutral SNI, accept-any
/// verifier, no HTTP phase. Survival is the only product.
pub async fn tls_foreign_attempt(
    ip: IpAddr,
    port: u16,
    neutral_name: &str,
    budget: Duration,
) -> AttemptResult {
    let obs: SharedObs = Arc::new(Mutex::new(CertObservation::default()));
    let fate = match tokio::time::timeout(
        budget,
        foreign_inner(ip, port, neutral_name, obs.clone()),
    )
    .await
    {
        Ok(f) => f,
        Err(_) => {
            if snapshot(&obs).stage_reached {
                AttemptFate::Foreign(ForeignOutcome::Survived)
            } else {
                AttemptFate::Foreign(ForeignOutcome::Timeout)
            }
        }
    };
    AttemptResult {
        cert: snapshot(&obs),
        fate,
    }
}

async fn foreign_inner(
    ip: IpAddr,
    port: u16,
    neutral_name: &str,
    obs: SharedObs,
) -> AttemptFate {
    let tcp = match TcpStream::connect(SocketAddr::new(ip, port)).await {
        Ok(s) => s,
        Err(_) => return AttemptFate::Foreign(ForeignOutcome::KilledPreCert),
    };
    let sni = match server_name(neutral_name) {
        Ok(n) => n,
        Err(_) => return AttemptFate::Foreign(ForeignOutcome::KilledPreCert),
    };
    let connector = TlsConnector::from(foreign_config(obs.clone()));
    match connector.connect(sni, tcp).await {
        Ok(_) => AttemptFate::Foreign(ForeignOutcome::Survived),
        Err(_) => {
            if snapshot(&obs).stage_reached {
                AttemptFate::Foreign(ForeignOutcome::Survived)
            } else {
                AttemptFate::Foreign(ForeignOutcome::KilledPreCert)
            }
        }
    }
}

/// Reads until the end of the first HTTP status line (or 64 KB / EOF /
/// error). Returns the status code, `None` when no complete status line
/// arrived — the connection died or stalled before an HTTP response.
async fn read_status_line<S>(stream: &mut S) -> Option<u16>
where
    S: AsyncReadExt + Unpin,
{
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n") {
            let line = &buf[..pos];
            return parse_status_line(line);
        }
        if buf.len() >= 64 * 1024 {
            return None;
        }
        match stream.read(&mut chunk).await {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn parse_status_line(line: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(line).ok()?;
    let mut parts = text.splitn(3, ' ');
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    //! The step-0 bench, rebuilt inside the project: the same four
    //! scenarios the design names, asserted against the taxonomy the
    //! connector must produce. Certificates are throwaway fixtures
    //! (test_certs/, a local CA generated for this bench only).
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn fixture_roots() -> Arc<RootCertStore> {
        let mut store = RootCertStore::empty();
        store
            .add(CertificateDer::from(
                &include_bytes!("test_certs/ca.der")[..],
            ))
            .unwrap();
        Arc::new(store)
    }

    fn server_certs(name: &str) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let (cert, key): (&[u8], &[u8]) = match name {
            "target.test" => (
                &include_bytes!("test_certs/target.test.der")[..],
                &include_bytes!("test_certs/target.test.key.der")[..],
            ),
            _ => (
                &include_bytes!("test_certs/other.test.der")[..],
                &include_bytes!("test_certs/other.test.key.der")[..],
            ),
        };
        (
            vec![CertificateDer::from(cert.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
        )
    }

    fn bind() -> (TcpListener, u16) {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    fn read_hello(s: &mut std::net::TcpStream) {
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut buf = vec![0u8; 16384];
        let _ = s.read(&mut buf);
    }

    /// (a) Death right after ClientHello, before any certificate:
    /// KilledPreCert. (Close with the client's flight unread yields
    /// RST on this path; EOF would map to the same outcome.)
    #[tokio::test]
    async fn killed_before_cert() {
        let (l, port) = bind();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                if let Ok(mut s) = stream {
                    read_hello(&mut s);
                    drop(s);
                }
            }
        });
        let r = tls_real_attempt(
            "127.0.0.1".parse().unwrap(),
            port,
            "target.test",
            fixture_roots(),
            Duration::from_secs(5),
        )
        .await;
        assert!(
            matches!(r.fate, AttemptFate::Real(TlsRealOutcome::KilledPreCert)),
            "fate: {:?}",
            std::mem::discriminant(&r.fate)
        );
        assert!(!r.cert.stage_reached);
    }

    /// (b) Death after the Certificate flight (TLS 1.2 manual server):
    /// PostCertKilled, with a valid-certificate observation.
    #[tokio::test]
    async fn killed_after_valid_cert() {
        let (l, port) = bind();
        std::thread::spawn(move || {
            let (chain, key) = server_certs("target.test");
            let config = rustls::ServerConfig::builder_with_protocol_versions(&[
                &rustls::version::TLS12,
            ])
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
            for stream in l.incoming() {
                if let Ok(mut s) = stream {
                    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut conn =
                        match rustls::ServerConnection::new(Arc::new(config.clone())) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                    if conn.read_tls(&mut s).is_err() {
                        continue;
                    }
                    if conn.process_new_packets().is_err() {
                        continue;
                    }
                    while conn.wants_write() {
                        if conn.write_tls(&mut s).is_err() {
                            break;
                        }
                    }
                    let _ = s.flush();
                    std::thread::sleep(Duration::from_millis(200));
                    drop(s);
                }
            }
        });
        let r = tls_real_attempt(
            "127.0.0.1".parse().unwrap(),
            port,
            "target.test",
            fixture_roots(),
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(
            r.fate,
            AttemptFate::Real(TlsRealOutcome::PostCertKilled)
        ));
        assert!(r.cert.stage_reached);
        assert_eq!(r.cert.valid, Some(true));
    }

    /// (c) Fatal alert instead of ServerHello: KilledPreCert.
    #[tokio::test]
    async fn alert_before_cert() {
        let (l, port) = bind();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                if let Ok(mut s) = stream {
                    read_hello(&mut s);
                    let alert: [u8; 7] = [0x15, 0x03, 0x03, 0x00, 0x02, 0x02, 0x28];
                    let _ = s.write_all(&alert);
                    let _ = s.flush();
                    std::thread::sleep(Duration::from_millis(100));
                    drop(s);
                }
            }
        });
        let r = tls_real_attempt(
            "127.0.0.1".parse().unwrap(),
            port,
            "target.test",
            fixture_roots(),
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(
            r.fate,
            AttemptFate::Real(TlsRealOutcome::KilledPreCert)
        ));
    }

    /// (d)+(e) Full handshake servers: wrong-name cert → CertInvalid;
    /// right cert → Http(200); the foreign leg survives both.
    #[tokio::test]
    async fn cert_invalid_and_http_and_foreign() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        for (cert_name, expect_real) in [
            ("other.test", TlsRealOutcome::CertInvalid),
            ("target.test", TlsRealOutcome::Http(200)),
        ] {
            let (chain, key) = server_certs(cert_name);
            let config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let acc = acceptor.clone();
                    tokio::spawn(async move {
                        if let Ok(mut tls) = acc.accept(stream).await {
                            let mut buf = vec![0u8; 4096];
                            let _ = tls.read(&mut buf).await;
                            let _ = tls
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                                .await;
                            let _ = tls.shutdown().await;
                        }
                    });
                }
            });

            let ip: IpAddr = "127.0.0.1".parse().unwrap();
            let r = tls_real_attempt(ip, port, "target.test", fixture_roots(), Duration::from_secs(5))
                .await;
            match expect_real {
                TlsRealOutcome::CertInvalid => {
                    assert!(matches!(r.fate, AttemptFate::Real(TlsRealOutcome::CertInvalid)));
                    assert_eq!(r.cert.valid, Some(false));
                }
                TlsRealOutcome::Http(code) => {
                    assert!(matches!(r.fate, AttemptFate::Real(TlsRealOutcome::Http(c)) if c == code));
                    assert_eq!(r.cert.valid, Some(true));
                }
                _ => unreachable!(),
            }

            // Foreign leg: with SNI www.wikipedia.org the same servers
            // must let the handshake live (they don't route on SNI).
            let f = tls_foreign_attempt(ip, port, "www.wikipedia.org", Duration::from_secs(5)).await;
            assert!(matches!(
                f.fate,
                AttemptFate::Foreign(ForeignOutcome::Survived)
            ));
        }
    }

    #[test]
    fn san_extraction_from_fixtures() {
        let target = CertificateDer::from(&include_bytes!("test_certs/target.test.der")[..]);
        assert_eq!(subject_line(&target).as_deref(), Some("target.test"));
        let other = CertificateDer::from(&include_bytes!("test_certs/other.test.der")[..]);
        assert_eq!(subject_line(&other).as_deref(), Some("other.test"));
        // The CA fixture has no SAN extension.
        let ca = CertificateDer::from(&include_bytes!("test_certs/ca.der")[..]);
        assert_eq!(subject_line(&ca), None);
    }

    #[test]
    fn status_line_parsing() {
        assert_eq!(parse_status_line(b"HTTP/1.1 200 OK"), Some(200));
        assert_eq!(parse_status_line(b"HTTP/1.1 503 Service Unavailable"), Some(503));
        assert_eq!(parse_status_line(b"HTTP/2 404"), Some(404));
        assert_eq!(parse_status_line(b"NOTHTTP 200"), None);
        assert_eq!(parse_status_line(b"HTTP/1.1"), None);
    }
}
