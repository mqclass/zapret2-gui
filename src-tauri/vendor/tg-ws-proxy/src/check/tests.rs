//! Unit tests for the listener probe: its request frame, its reply check, and
//! how it classifies what a listener sends back.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{FRAME_HEADER_LEN, ProbeStatus, build_req_pq_multi, probe_listener, reply_is_res_pq};
use crate::crypto;

/// The frame a peer reads: a 4-byte length prefix, 8 zero bytes where a session
/// would carry `auth_key_id`, the message id, the body length, and the body
/// padded to a multiple of 4.
#[test]
fn req_pq_multi_is_a_padded_intermediate_frame() {
    let frame = build_req_pq_multi();

    assert_eq!(frame.len() % 4, 0, "padded to a multiple of 4");

    let declared = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
    assert_eq!(
        declared + 4,
        frame.len(),
        "the length prefix covers the frame"
    );

    assert_eq!(
        &frame[4..12],
        &[0u8; 8],
        "no session yet: auth_key_id is zero"
    );

    let body_len = u32::from_le_bytes(frame[20..24].try_into().unwrap()) as usize;
    assert_eq!(
        body_len,
        4 + 16,
        "the body is the constructor and its nonce"
    );
    assert_eq!(&frame[24..28], &0xbe7e_8ef1u32.to_le_bytes());
    assert!(
        frame[28..44].iter().any(|byte| *byte != 0),
        "the nonce is random, not zeroed"
    );
}

/// `resPQ` with a zero `auth_key_id` is the answer we want; another
/// constructor, a session key or a short read is not.
#[test]
fn only_a_zero_key_res_pq_reply_passes() {
    let mut reply = vec![0u8; FRAME_HEADER_LEN];
    reply[24..28].copy_from_slice(&0x0516_2463u32.to_le_bytes());
    assert!(reply_is_res_pq(&reply));

    reply[24..28].copy_from_slice(&0xbe7e_8ef1u32.to_le_bytes());
    assert!(!reply_is_res_pq(&reply), "another constructor is not resPQ");

    let mut reply = vec![0u8; FRAME_HEADER_LEN];
    reply[24..28].copy_from_slice(&0x0516_2463u32.to_le_bytes());
    reply[4] = 1;
    assert!(
        !reply_is_res_pq(&reply),
        "a session key means it is not a resPQ"
    );

    assert!(
        !reply_is_res_pq(&reply[..FRAME_HEADER_LEN - 1]),
        "short read"
    );
    assert!(!reply_is_res_pq(&[]));
}

/// A transport error is a framed packet — a 4-byte length of 4 and the negative
/// code — so it must be read as one, not mistaken for a truncated frame and
/// blamed on the routing.
#[tokio::test]
async fn a_framed_transport_error_is_reported_as_one() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let secret = [0x11u8; 16];

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut init = [0u8; crypto::HANDSHAKE_LEN];
        stream.read_exact(&mut init).await.unwrap();
        let info = crypto::parse_handshake(&init, &secret).expect("client handshake parses");
        let relay_init = crypto::generate_relay_init(info.proto, info.dc_id as i16);
        let mut ciphers =
            crypto::build_connection_ciphers(&info.prekey_and_iv, &secret, &relay_init);

        // Drain the whole request, so closing afterwards cannot reset the
        // connection and discard the error still in the probe's buffer.
        let mut request = [0u8; 44];
        stream.read_exact(&mut request).await.unwrap();

        let mut reply = [0u8; 8];
        reply[..4].copy_from_slice(&4u32.to_le_bytes());
        reply[4..8].copy_from_slice(&(-404i32).to_le_bytes());
        crypto::apply_keystream(&mut ciphers.clt_enc, &mut reply);
        stream.write_all(&reply).await.unwrap();
    });

    match probe_listener(addr, &secret, 2, Duration::from_secs(5)).await {
        ProbeStatus::Fail(reason) => assert!(
            reason.contains("transport error: -404"),
            "unexpected reason: {reason}"
        ),
        ProbeStatus::Ok(elapsed) => panic!("expected a failure, got OK in {elapsed:?}"),
    }

    server.await.unwrap();
}
