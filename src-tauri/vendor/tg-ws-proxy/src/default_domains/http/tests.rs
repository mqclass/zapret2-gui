use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::*;
use crate::default_domains::{DOMAINS_URL_HOST, DOMAINS_URL_PATH, parse_domain_list};

#[test]
fn content_length_rejects_conflicting_duplicates() {
    let mut headers = [
        httparse::Header {
            name: "Content-Length",
            value: b"10",
        },
        httparse::Header {
            name: "Content-Length",
            value: b"999",
        },
    ];

    let err = content_length(&headers).unwrap_err();

    assert!(err.contains("conflicting"));
    headers[1].value = b"10";
    assert_eq!(content_length(&headers).unwrap(), Some(10));
}

#[tokio::test]
async fn read_http_response_body_rejects_conflicting_content_length_with_chunked() {
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n0\r\n\r\n"
            .as_slice();

    let err = read_http_response_body(&mut response).await.unwrap_err();

    assert!(err.contains("conflicting"));
}

#[tokio::test]
async fn read_http_response_body_stops_at_content_length_without_eof() {
    let (mut client, mut server) = tokio::io::duplex(1024);
    let writer = tokio::spawn(async move {
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nvirkgj.com\nextra")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let body = read_http_response_body(&mut client).await.unwrap();

    assert_eq!(body, "virkgj.com\n");
    writer.abort();
    assert!(writer.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn read_http_response_body_accepts_chunked_body() {
    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nvirkgj\r\n5\r\n.com\n\r\n0\r\n\r\n".as_slice();

    let body = read_http_response_body(&mut response).await.unwrap();

    assert_eq!(body, "virkgj.com\n");
}

#[tokio::test]
async fn read_http_response_body_rejects_truncated_chunked_terminator() {
    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n".as_slice();

    let err = read_http_response_body(&mut response).await.unwrap_err();

    assert!(err.contains("unexpected EOF"));
}

#[tokio::test]
async fn read_http_response_body_accepts_more_than_32_headers() {
    let mut response = b"HTTP/1.1 200 OK\r\n".to_vec();
    for i in 0..40 {
        response.extend_from_slice(format!("X-Test-{i}: value\r\n").as_bytes());
    }
    response.extend_from_slice(b"Content-Length: 11\r\n\r\nvirkgj.com\n");

    let body = read_http_response_body(&mut response.as_slice())
        .await
        .unwrap();

    assert_eq!(body, "virkgj.com\n");
}

#[tokio::test]
async fn read_http_response_body_rejects_non_200_status() {
    let mut response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".as_slice();

    let err = read_http_response_body(&mut response).await.unwrap_err();

    assert!(err.contains("HTTP status 404"));
}

#[tokio::test]
async fn read_http_response_body_rejects_oversized_content_length() {
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\n".as_slice();

    let err = read_http_response_body(&mut response).await.unwrap_err();

    assert!(err.contains("exceeds limit"));
}

#[tokio::test]
async fn read_http_response_body_accepts_eof_delimited_body_at_limit() {
    let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    response.extend(vec![b'a'; MAX_BODY_BYTES]);

    let body = read_http_response_body(&mut response.as_slice())
        .await
        .unwrap();

    assert_eq!(body.len(), MAX_BODY_BYTES);
}

#[tokio::test]
async fn https_get_fetches_domain_list_through_outbound_proxy() {
    install_rustls_provider();

    let (cert, key) = test_certificate();
    let (server_addr, server_task) = tls_domain_list_server(cert.clone(), key).await;
    let (proxy_addr, proxy_task) = mapping_http_proxy(server_addr).await;
    let outbound =
        OutboundConnector::from_config(Some(&format!("http://{proxy_addr}")), None, false).unwrap();

    let body = https_get_with_tls_config(
        DOMAINS_URL_HOST,
        DOMAINS_URL_PATH,
        &outbound,
        test_tls_config(cert),
    )
    .await
    .unwrap();

    assert_eq!(parse_domain_list(&body), vec!["pclead.co.uk"]);
    let request = await_string_task(proxy_task).await;
    assert!(request.starts_with("CONNECT raw.githubusercontent.com:443 HTTP/1.1"));
    await_unit_task(server_task).await;
}

fn test_certificate() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let cert = rcgen::generate_simple_self_signed(vec![DOMAINS_URL_HOST.to_string()]).unwrap();
    let cert_der = CertificateDer::from(cert.serialize_der().unwrap());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    (cert_der, key_der)
}

fn test_tls_config(cert: CertificateDer<'static>) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).unwrap();
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

async fn tls_domain_list_server(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(stream).await.unwrap();
        let request = read_http_request(&mut stream).await;
        assert!(request.starts_with(&format!("GET {DOMAINS_URL_PATH} HTTP/1.1")));
        assert!(request.contains(&format!("Host: {DOMAINS_URL_HOST}")));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nvirkgj\r\n5\r\n.com\n\r\n0\r\n\r\n",
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    (addr, task)
}

async fn mapping_http_proxy(
    target: std::net::SocketAddr,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut inbound, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut inbound).await;
        inbound.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();

        let outbound = TcpStream::connect(target).await.unwrap();
        let (mut ri, mut wi) = inbound.split();
        let (mut ro, mut wo) = tokio::io::split(outbound);
        let _ = tokio::join!(
            tokio::io::copy(&mut ri, &mut wo),
            tokio::io::copy(&mut ro, &mut wi)
        );

        request
    });
    (addr, task)
}

async fn read_http_request<T>(stream: &mut T) -> String
where
    T: AsyncRead + Unpin,
{
    let mut request = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0);
        request.extend_from_slice(&buf[..n]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).to_string()
}

async fn await_string_task(task: tokio::task::JoinHandle<String>) -> String {
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("test helper task timed out")
        .expect("test helper task panicked")
}

async fn await_unit_task(task: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("test helper task timed out")
        .expect("test helper task panicked");
}

fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
