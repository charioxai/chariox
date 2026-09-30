use super::{test_server::TestDns, DnsConfig, BUDGET, PER_SERVER};
use crate::runtime::app_http::{limits::HttpLimits, policy::fixture_get, HttpError};
use hickory_resolver::proto::{op::ResponseCode, rr::RecordType};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};
use tokio::{
    sync::watch,
    time::{Duration, Instant},
};

async fn resolve(dns: &DnsConfig, url: &str) -> Result<Vec<SocketAddr>, HttpError> {
    resolve_within(dns, url, Duration::from_secs(30)).await
}

async fn resolve_within(
    dns: &DnsConfig,
    url: &str,
    deadline: Duration,
) -> Result<Vec<SocketAddr>, HttpError> {
    let limits = HttpLimits::default();
    let (_stop, stopped) = watch::channel(false);
    dns.resolve(
        &fixture_get(url),
        Instant::now() + deadline,
        stopped,
        limits.acquire("alice", "app").unwrap(),
    )
    .await
}

async fn public() -> TestDns {
    TestDns::start(|_, _, _| Some(vec!["8.8.8.8".parse().unwrap()])).await
}

fn answered() -> Vec<SocketAddr> {
    vec!["8.8.8.8:443".parse().unwrap()]
}

/// Timing margin for a loaded test machine.
const SLACK: Duration = Duration::from_millis(900);

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

#[tokio::test]
async fn a_blackholed_first_server_gives_way_to_the_next_after_its_turn() {
    let (silent, good) = (TestDns::silent().await, public().await);
    let config = DnsConfig::fixture_servers(&[silent.address(), good.address()]);
    let start = Instant::now();
    assert_eq!(
        resolve(&config, "https://public.test/").await.unwrap(),
        answered()
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed >= PER_SERVER && elapsed < PER_SERVER + SLACK,
        "{elapsed:?}"
    );
    assert!(silent.queries("public.test.", RecordType::A) >= 1);
    assert!(good.asked_between(start, PER_SERVER, PER_SERVER + SLACK));
}

#[tokio::test]
async fn an_error_code_gives_way_to_the_next_server_at_once() {
    for code in [
        ResponseCode::Refused,
        ResponseCode::NotImp,
        ResponseCode::ServFail,
    ] {
        let (failing, good) = (TestDns::failing(code).await, public().await);
        let config = DnsConfig::fixture_servers(&[failing.address(), good.address()]);
        let start = Instant::now();
        assert_eq!(
            resolve(&config, "https://public.test/").await.unwrap(),
            answered(),
            "{code:?}"
        );
        assert!(start.elapsed() < SLACK, "{code:?}: {:?}", start.elapsed());
        assert!(failing.total_queries() >= 1, "{code:?}");
    }
}

#[tokio::test]
async fn a_lone_server_is_asked_again_before_the_lookup_fails() {
    let silent = TestDns::silent().await;
    let config = DnsConfig::fixture_servers(&[silent.address()]);
    let start = Instant::now();
    assert_eq!(
        resolve(&config, "https://public.test/").await,
        Err(HttpError::Network)
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed >= 2 * PER_SERVER && elapsed < 2 * PER_SERVER + SLACK,
        "{elapsed:?}"
    );
    assert!(silent.asked_between(start, Duration::ZERO, PER_SERVER));
    assert!(silent.asked_between(start, PER_SERVER, 2 * PER_SERVER));
}

#[tokio::test]
async fn every_server_down_fails_as_network_within_the_total_budget() {
    let (first, second) = (TestDns::silent().await, TestDns::silent().await);
    let config = DnsConfig::fixture_servers(&[first.address(), second.address()]);
    let start = Instant::now();
    assert_eq!(
        resolve(&config, "https://public.test/").await,
        Err(HttpError::Network)
    );
    let elapsed = start.elapsed();
    assert!(elapsed >= BUDGET && elapsed < BUDGET + SLACK, "{elapsed:?}");
    // In order, one turn each, then the first again until the budget ends.
    assert!(first.asked_between(start, Duration::ZERO, PER_SERVER));
    assert!(second.asked_between(start, PER_SERVER, 2 * PER_SERVER));
    assert!(first.asked_between(start, 2 * PER_SERVER, BUDGET));
    assert!(!second.asked_between(
        start,
        2 * PER_SERVER + Duration::from_millis(200),
        BUDGET + SLACK
    ));
}

#[tokio::test]
async fn a_missing_name_is_final_and_the_request_deadline_still_wins() {
    let (missing, good) = (TestDns::start(|_, _, _| None).await, public().await);
    let config = DnsConfig::fixture_servers(&[missing.address(), good.address()]);
    let start = Instant::now();
    assert_eq!(
        resolve(&config, "https://public.test/").await,
        Err(HttpError::Network)
    );
    assert!(start.elapsed() < SLACK);
    assert_eq!(
        good.total_queries(),
        0,
        "NXDOMAIN is an answer, not a failure"
    );

    let (silent, good) = (TestDns::silent().await, public().await);
    let config = DnsConfig::fixture_servers(&[silent.address(), good.address()]);
    let start = Instant::now();
    assert_eq!(
        resolve_within(&config, "https://public.test/", Duration::from_millis(500)).await,
        Err(HttpError::Deadline)
    );
    assert!(start.elapsed() < Duration::from_millis(500) + SLACK);
    assert_eq!(good.total_queries(), 0);
}

#[tokio::test]
async fn cancellation_ends_a_turn_at_once() {
    let silent = TestDns::silent().await;
    let config = DnsConfig::fixture_servers(&[silent.address()]);
    let limits = HttpLimits::default();
    let (stop, stopped) = watch::channel(false);
    let target = fixture_get("https://public.test/");
    let start = Instant::now();
    let lookup = config.resolve(
        &target,
        Instant::now() + Duration::from_secs(30),
        stopped,
        limits.acquire("alice", "app").unwrap(),
    );
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.send(true).unwrap();
    };
    let (result, ()) = tokio::join!(lookup, cancel);
    assert_eq!(result, Err(HttpError::Cancelled));
    assert!(start.elapsed() < Duration::from_millis(300) + SLACK);
}
