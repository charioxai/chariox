use std::{
    net::{Ipv4Addr, Ipv6Addr},
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;

const V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const V6: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
const DEAD_FAMILY_DELAY: Duration = Duration::from_secs(3);

fn answers(
    v4_delay: Duration,
    v6_delay: Duration,
) -> impl Fn(&str, u16, Family) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static {
    move |_, port, family| match family {
        Family::V4 => {
            std::thread::sleep(v4_delay);
            Ok(vec![SocketAddr::new(V4.into(), port)])
        }
        Family::V6 => {
            std::thread::sleep(v6_delay);
            Ok(vec![SocketAddr::new(V6.into(), port)])
        }
    }
}

#[test]
fn a_dead_aaaa_lookup_does_not_delay_the_a_answer() {
    let resolver = Resolver::new(answers(Duration::ZERO, DEAD_FAMILY_DELAY));
    let started = Instant::now();

    let addresses = resolver.resolve("cloud.example:443").unwrap();

    assert!(started.elapsed() < Duration::from_millis(200));
    assert_eq!(addresses, vec![SocketAddr::new(V4.into(), 443)]);
}

#[test]
fn a_dead_a_lookup_delays_a_lone_aaaa_answer_by_one_address_attempt_at_most() {
    let resolver = Resolver::new(answers(DEAD_FAMILY_DELAY, Duration::ZERO));
    let started = Instant::now();

    let addresses = resolver.resolve("cloud.example:443").unwrap();

    let elapsed = started.elapsed();
    assert!(
        elapsed >= A_AFTER_AAAA_WAIT && elapsed < A_AFTER_AAAA_WAIT + Duration::from_millis(200),
        "returned after {elapsed:?}"
    );
    assert_eq!(addresses, vec![SocketAddr::new(V6.into(), 443)]);
}

// An AAAA answer can name an unreachable path while the A answer that works
// is still on its way; the dial must get both.
#[test]
fn a_lone_aaaa_answer_waits_for_an_a_answer_on_its_way() {
    let resolver = Resolver::new(answers(Duration::from_millis(150), Duration::ZERO));

    let addresses = resolver.resolve("cloud.example:443").unwrap();

    assert_eq!(
        addresses,
        vec![
            SocketAddr::new(V6.into(), 443),
            SocketAddr::new(V4.into(), 443)
        ]
    );
}

#[test]
fn a_second_family_inside_the_stagger_is_used_with_the_first() {
    let resolver = Resolver::new(answers(Duration::ZERO, Duration::from_millis(15)));

    let addresses = resolver.resolve("cloud.example:443").unwrap();

    assert_eq!(
        addresses,
        vec![
            SocketAddr::new(V6.into(), 443),
            SocketAddr::new(V4.into(), 443)
        ]
    );
}

#[test]
fn later_calls_use_the_cache_and_a_late_second_family() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&lookups);
    let inner = answers(Duration::ZERO, Duration::from_millis(300));
    let resolver = Resolver::new(move |host, port, family| {
        counted.fetch_add(1, Ordering::SeqCst);
        inner(host, port, family)
    });
    assert_eq!(resolver.resolve("cloud.example:443").unwrap().len(), 1);

    let started = Instant::now();
    let again = resolver.resolve("cloud.example:443").unwrap();
    assert!(started.elapsed() < Duration::from_millis(20));
    assert_eq!(again.len(), 1);

    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(resolver.resolve("cloud.example:443").unwrap().len(), 2);
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
}

#[test]
fn a_hanging_lookup_is_not_started_again_while_it_is_pending() {
    let v6_lookups = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&v6_lookups);
    let inner = answers(Duration::ZERO, DEAD_FAMILY_DELAY);
    let resolver = Resolver::new(move |host, port, family| {
        if family == Family::V6 {
            counted.fetch_add(1, Ordering::SeqCst);
        }
        inner(host, port, family)
    });

    for _ in 0..5 {
        resolver.resolve("cloud.example:443").unwrap();
    }

    assert_eq!(v6_lookups.load(Ordering::SeqCst), 1);
}

#[test]
fn failures_report_the_error_of_the_lookup() {
    let resolver = Resolver::new(|_, _, _| Err(io::Error::other("no such host")));

    let error = resolver.resolve("missing.example:443").unwrap_err();

    assert!(error.to_string().contains("no such host"));
}

#[test]
fn a_failed_host_is_looked_up_again_on_the_next_call() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&lookups);
    let resolver = Resolver::new(move |_, port, family| {
        if counted.fetch_add(1, Ordering::SeqCst) < 2 {
            return Err(io::Error::other("network is down"));
        }
        match family {
            Family::V4 => Ok(vec![SocketAddr::new(V4.into(), port)]),
            Family::V6 => Ok(Vec::new()),
        }
    });
    resolver.resolve("cloud.example:443").unwrap_err();

    assert_eq!(
        resolver.resolve("cloud.example:443").unwrap(),
        vec![SocketAddr::new(V4.into(), 443)]
    );
}

#[test]
fn numeric_hosts_need_no_lookup() {
    let resolver = Resolver::new(|_, _, _| panic!("numeric hosts must not be looked up"));

    assert_eq!(
        resolver.resolve("127.0.0.1:8080").unwrap(),
        vec!["127.0.0.1:8080".parse().unwrap()]
    );
    assert_eq!(
        resolver.resolve("[::1]:8080").unwrap(),
        vec!["[::1]:8080".parse().unwrap()]
    );
}

#[test]
fn the_system_lookup_resolves_localhost_per_family() {
    let v4 = system_lookup("localhost", 80, Family::V4).unwrap();

    assert!(!v4.is_empty() && v4.iter().all(SocketAddr::is_ipv4));
    assert!(system_lookup("localhost", 80, Family::V6)
        .unwrap_or_default()
        .iter()
        .all(SocketAddr::is_ipv6));
}

#[test]
fn the_system_lookup_keeps_the_scope_of_a_link_local_address() {
    #[cfg(target_os = "macos")]
    let interface = c"lo0";
    #[cfg(not(target_os = "macos"))]
    let interface = c"lo";
    // SAFETY: the platform interface name is a static NUL-terminated string.
    let scope = unsafe { libc::if_nametoindex(interface.as_ptr()) };
    assert_ne!(scope, 0);

    let addresses = system_lookup(&format!("fe80::1%{scope}"), 443, Family::V6).unwrap();

    let [SocketAddr::V6(address)] = addresses.as_slice() else {
        panic!("expected one IPv6 address, got {addresses:?}");
    };
    assert_eq!(address.scope_id(), scope);
    assert_eq!(address.port(), 443);
}
