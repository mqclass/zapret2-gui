use super::config::NoProxy;

#[test]
fn no_proxy_matches_without_constructing_a_url() {
    let matcher = NoProxy::from_validated(
        "localhost,.example.com,*.internal,example.net:443,127.0.0.0/8,[2001:db8::1]:8443",
    );

    assert!(matcher.matches("api.localhost", 80));
    assert!(matcher.matches("cdn.example.com", 443));
    assert!(!matcher.matches("example.com", 443));
    assert!(matcher.matches("node.internal", 443));
    assert!(matcher.matches("example.net", 443));
    assert!(!matcher.matches("example.net", 80));
    assert!(matcher.matches("127.20.30.40", 1234));
    assert!(matcher.matches("2001:db8::1", 8443));
    assert!(!matcher.matches("2001:db8::1", 443));
}

#[test]
fn no_proxy_preserves_implicit_loopback_and_case_insensitive_hosts() {
    let matcher = NoProxy::from_validated("unrelated.example");
    assert!(matcher.matches("127.0.0.1", 443));
    assert!(matcher.matches("::1", 443));

    let matcher = NoProxy::from_validated("Example.COM");
    assert!(matcher.matches("example.com", 443));
    assert!(matcher.matches("API.EXAMPLE.COM", 443));
}
