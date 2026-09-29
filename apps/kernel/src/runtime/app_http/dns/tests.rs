use super::{test_server::TestDns, DnsConfig};
use crate::runtime::app_http::{limits::HttpLimits, policy::fixture_get, HttpError};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};
use tokio::{
    sync::watch,
    time::{Duration, Instant},
};

async fn resolve(dns: &DnsConfig, url: &str) -> Result<Vec<SocketAddr>, HttpError> {
    let limits = HttpLimits::default();
    let (_stop, stopped) = watch::channel(false);
    dns.resolve(
        &fixture_get(url),
        Instant::now() + Duration::from_secs(5),
        stopped,
        limits.acquire("alice", "app").unwrap(),
    )
    .await
}

/// Each name answers one special-purpose address through real DNS. The
/// answer itself is refused as a destination; no connection is attempted.
const SPECIAL: &[(&str, &str)] = &[
    // The spellings the live drill uses (sslip.io encodes the address).
    ("0--1.sslip.io", "::1"),
    ("0--ffff-7f00-1.sslip.io", "::ffff:127.0.0.1"),
    ("0--ffff-a00-1.sslip.io", "::ffff:10.0.0.1"),
    ("0--ffff-a9fe-a9fe.sslip.io", "::ffff:169.254.169.254"),
    ("0--0.sslip.io", "::"),
    ("mapped-public.test", "::ffff:8.8.8.8"),
    ("translated.test", "::ffff:0:7f00:1"),
    ("compatible.test", "::7f00:1"),
    ("nat64-loopback.test", "64:ff9b::7f00:1"),
    ("nat64-metadata.test", "64:ff9b::a9fe:a9fe"),
    ("nat64-local-use.test", "64:ff9b:1::a00:1"),
    ("sixtofour-loopback.test", "2002:7f00:1::1"),
    ("sixtofour-metadata.test", "2002:a9fe:a9fe::1"),
    ("teredo.test", "2001:0:4136:e378:8000:63bf:3fff:fdd2"),
    ("metadata-ula.test", "fd00:ec2::254"),
    ("link-local.test", "fe80::1"),
    ("site-local.test", "fec0::1"),
    ("loopback.test", "127.0.0.1"),
    ("unspecified.test", "0.0.0.0"),
    ("private.test", "10.0.0.1"),
    ("metadata.test", "169.254.169.254"),
    ("shared.test", "100.64.0.1"),
];

#[tokio::test]
async fn names_answering_loopback_mapped_embedded_or_private_addresses_are_destination_denials() {
    let mut answers: HashMap<String, Vec<IpAddr>> = SPECIAL
        .iter()
        .map(|(name, address)| (format!("{name}."), vec![address.parse().unwrap()]))
        .collect();
    // One private answer beside a public one still refuses the whole name.
    answers.insert(
        "mixed.test.".into(),
        vec!["8.8.8.8".parse().unwrap(), "::1".parse().unwrap()],
    );
    answers.insert(
        "public.test.".into(),
        vec![
            "8.8.8.8".parse().unwrap(),
            "2606:4700:4700::1111".parse().unwrap(),
        ],
    );
    let dns = TestDns::start(move |name, _, _| answers.get(name).cloned()).await;
    let config = dns.config();
    for (name, address) in SPECIAL {
        assert_eq!(
            resolve(&config, &format!("https://{name}/")).await,
            Err(HttpError::Destination),
            "{name} answering {address}"
        );
    }
    assert_eq!(
        resolve(&config, "https://mixed.test/").await,
        Err(HttpError::Destination)
    );
    assert_eq!(
        resolve(&config, "https://public.test/").await.unwrap(),
        vec![
            "8.8.8.8:443".parse().unwrap(),
            "[2606:4700:4700::1111]:443".parse().unwrap()
        ]
    );
    assert_eq!(
        resolve(&config, "https://missing.test/").await,
        Err(HttpError::Network)
    );
}

#[tokio::test]
async fn special_ip_literals_are_denied_without_a_query() {
    let dns = TestDns::start(|_, _, _| Some(vec!["::1".parse().unwrap()])).await;
    let config = dns.config();
    for url in [
        "https://[::1]/",
        "https://[::]/",
        "https://[::ffff:127.0.0.1]/",
        "https://[::ffff:169.254.169.254]/",
        "https://[64:ff9b::a9fe:a9fe]/",
        "https://[2002:7f00:1::1]/",
        "https://127.0.0.1/",
        "https://0.0.0.0/",
        "https://169.254.169.254/",
    ] {
        assert_eq!(
            resolve(&config, url).await,
            Err(HttpError::Destination),
            "{url}"
        );
    }
    assert_eq!(dns.total_queries(), 0);
}
