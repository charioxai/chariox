use super::*;

fn rules() -> Rules {
    Rules {
        destinations: BTreeMap::from([
            (
                "https://api.example.com".into(),
                BTreeSet::from([HttpMethod::Get, HttpMethod::Post]),
            ),
            (
                "https://pay.example.com".into(),
                BTreeSet::from([HttpMethod::Get, HttpMethod::Post]),
            ),
        ]),
        protected_origins: BTreeSet::from(["https://pay.example.com".into()]),
    }
}

#[test]
fn exact_signed_origin_and_method_are_required() {
    let rules = rules();
    assert!(rules
        .target("https://api.example.com/v1?q=hello", "POST", &[])
        .is_ok());
    for url in [
        "https://api.example.com.evil.test/",
        "http://api.example.com/",
        "https://api.example.com:444/",
        "https://elsewhere.example/",
    ] {
        assert!(matches!(
            rules.target(url, "GET", &[]),
            Err(HttpError::Destination)
        ));
    }
    assert!(matches!(
        rules.target("https://api.example.com/", "DELETE", &[]),
        Err(HttpError::Destination)
    ));
    for method in ["get", "CONNECT", "TRACE", "GET\r\n"] {
        assert!(matches!(
            rules.target("https://api.example.com/", method, &[]),
            Err(HttpError::Invalid)
        ));
    }
}

#[test]
fn credentials_fragments_and_control_bytes_are_rejected() {
    for url in [
        "https://user:secret@api.example.com/",
        "https://api.example.com/#private",
        "https://api.example.com/a\nb",
        "file:///etc/passwd",
        "data:text/plain,hello",
    ] {
        assert!(matches!(
            rules().target(url, "GET", &[]),
            Err(HttpError::Invalid)
        ));
    }
}

#[test]
fn anonymous_transport_cannot_alias_a_declared_critical_service() {
    for path in [
        "/",
        "/harmless",
        "/%70ay",
        "/pay?method=POST",
        "/other/../pay",
    ] {
        assert!(matches!(
            rules().target(&format!("https://pay.example.com{path}"), "GET", &[]),
            Err(HttpError::ProtectedEffect)
        ));
    }
}

#[test]
fn transport_headers_cannot_override_routing_framing_or_credentials() {
    for name in [
        "HOST",
        "Authorization",
        "Cookie",
        "Proxy-Authorization",
        "Connection",
        "Content-Length",
        "Transfer-Encoding",
        "Expect",
        "Accept-Encoding",
        "Sec-WebSocket-Key",
        "X-HTTP-Method-Override",
    ] {
        assert!(matches!(
            request_headers(&[(name.into(), "value".into())]),
            Err(HttpError::Invalid)
        ));
    }
    assert!(matches!(
        request_headers(&[("x-safe".into(), "a\r\nb: c".into())]),
        Err(HttpError::Invalid)
    ));
    let headers = request_headers(&[
        (
            "Content-Type".into(),
            "multipart/form-data; boundary=fixture".into(),
        ),
        ("x-tag".into(), "one".into()),
        ("x-tag".into(), "two".into()),
    ])
    .unwrap();
    assert_eq!(headers.get_all("x-tag").iter().count(), 2);
    assert!(matches!(
        request_headers(&vec![("x".into(), "".into()); MAX_HEADERS + 1]),
        Err(HttpError::Limit)
    ));
    assert!(matches!(
        request_headers(&[("x".into(), "a".repeat(MAX_HEADER_BYTES))]),
        Err(HttpError::Limit)
    ));
}

#[test]
fn mixed_dns_private_ipv6_mapped_and_excess_answers_are_denied() {
    let target = rules()
        .target("https://api.example.com/", "GET", &[])
        .unwrap();
    let public: SocketAddr = "8.8.8.8:443".parse().unwrap();
    for denied in [
        "127.0.0.1:443",
        "169.254.169.254:443",
        "10.0.0.1:443",
        "[::1]:443",
        "[::ffff:8.8.8.8]:443",
        "[fc00::1]:443",
        "8.8.8.8:80",
    ] {
        assert!(matches!(
            target.validate_addresses([public, denied.parse().unwrap()]),
            Err(HttpError::Destination)
        ));
    }
    assert!(matches!(
        target.validate_addresses(vec![public; MAX_RESOLVED_ADDRESSES + 1]),
        Err(HttpError::Limit)
    ));
    assert_eq!(
        target.validate_addresses([public, public]).unwrap(),
        vec![public]
    );
}

#[test]
fn connected_socket_must_match_the_checked_numeric_endpoint() {
    let target = rules()
        .target("https://api.example.com/", "GET", &[])
        .unwrap();
    let selected = "8.8.8.8:443".parse().unwrap();
    target.require_connected(selected, selected).unwrap();
    for actual in ["127.0.0.1:443", "1.1.1.1:443", "8.8.8.8:80"] {
        assert!(matches!(
            target.require_connected(selected, actual.parse().unwrap()),
            Err(HttpError::Destination)
        ));
    }
}
