//! Cloud answers 429 with how long to wait. Every Cloud call of this kernel waits that long,
//! and asks again later when it is refused again, instead of retrying into the limit.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ureq::{Error, Middleware, MiddlewareNext, Request, Response};

const FIRST_PAUSE: Duration = Duration::from_secs(1);
/// Longest pause the back-off adds by itself; Cloud's own `Retry-After` may ask for more.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const MAX_PAUSE: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Pause {
    until: Option<Instant>,
    /// Refusals in a row since the last answer that was not one.
    refusals: u32,
}

/// Added to the agent of every Cloud API call. The pause is shared by all calls to one Cloud,
/// whichever feature makes them.
pub(super) struct CloudRateLimit;

fn pauses() -> &'static Mutex<HashMap<String, Pause>> {
    static PAUSES: OnceLock<Mutex<HashMap<String, Pause>>> = OnceLock::new();
    PAUSES.get_or_init(Mutex::default)
}

impl Middleware for CloudRateLimit {
    fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error> {
        let Some(cloud) = origin(request.url()) else {
            return next.handle(request);
        };
        if let Some(wait) = remaining(&cloud) {
            return Ok(refusal(wait));
        }
        let result = next.handle(request);
        match &result {
            Ok(response) if response.status() == 429 => refused(&cloud, retry_after(response)),
            Ok(_) => answered(&cloud),
            Err(_) => {}
        }
        result
    }
}

fn origin(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    Some(format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str()?,
        url.port_or_known_default()?
    ))
}

fn retry_after(response: &Response) -> Option<Duration> {
    let seconds = response.header("retry-after")?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_PAUSE))
}

fn remaining(cloud: &str) -> Option<Duration> {
    let pauses = pauses()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let until = pauses.get(cloud)?.until?;
    until.checked_duration_since(Instant::now())
}

fn refused(cloud: &str, retry_after: Option<Duration>) {
    let mut pauses = pauses()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let pause = pauses.entry(cloud.to_string()).or_default();
    let backoff = FIRST_PAUSE
        .saturating_mul(1_u32 << pause.refusals.min(6))
        .min(MAX_BACKOFF);
    pause.refusals = pause.refusals.saturating_add(1);
    pause.until = Some(Instant::now() + retry_after.unwrap_or_default().max(backoff));
}

fn answered(cloud: &str) {
    let mut pauses = pauses()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(pause) = pauses.get_mut(cloud) {
        *pause = Pause::default();
    }
}

/// What Cloud itself would have said, so callers read one kind of refusal; no request was sent.
fn refusal(wait: Duration) -> Response {
    let seconds = wait.as_secs_f64().ceil().max(1.0) as u64;
    let unit = if seconds == 1 { "second" } else { "seconds" };
    let body = format!(
        r#"{{"error":{{"code":"rate_limited","message":"Rate limit exceeded, retry in {seconds} {unit}"}}}}"#
    );
    format!(
        "HTTP/1.1 429 Too Many Requests\r\nretry-after: {seconds}\r\ncontent-type: application/json\r\n\r\n{body}"
    )
    .parse()
    .expect("a fixed 429 response parses")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A Cloud that refuses the first `refusals` requests with `Retry-After: 2`.
    fn cloud(refusals: usize) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer);
                let reply = if counted.fetch_add(1, Ordering::SeqCst) < refusals {
                    "HTTP/1.1 429 Too Many Requests\r\nretry-after: 2\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"code\":\"rate_limited\",\"message\":\"Rate limit exceeded, retry in 2 seconds\"}}"
                } else {
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}"
                };
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (url, seen)
    }

    fn call(url: &str) -> Result<Response, Error> {
        super::super::http::cloud_agent_builder()
            .build()
            .get(&format!("{url}/relay/targets"))
            .call()
    }

    // The 429 storm: Cloud says "retry in 2 seconds"; a kernel that asks again before that is
    // sending exactly the traffic that keeps the limit tripped.
    #[test]
    fn a_refused_call_keeps_every_cloud_call_quiet_until_retry_after() {
        let (url, seen) = cloud(1);
        assert!(matches!(call(&url), Err(Error::Status(429, _))));
        for _ in 0..20 {
            let Err(Error::Status(429, response)) = call(&url) else {
                panic!("a call inside the pause must be refused locally");
            };
            let body = response.into_string().unwrap();
            assert!(
                body.contains("rate_limited") && body.contains("retry in"),
                "{body}"
            );
        }
        assert_eq!(
            seen.load(Ordering::SeqCst),
            1,
            "no request reached Cloud during the pause"
        );
        std::thread::sleep(Duration::from_millis(2_200));
        assert!(
            call(&url).is_ok(),
            "the first call after the pause goes through"
        );
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    // The same storm through the kernel's own directory read: eight reads, one request.
    #[tokio::test]
    async fn the_kernel_directory_is_not_read_again_inside_the_pause() {
        let (api_url, seen) = cloud(1);
        let profile = crate::config::PersistedCloudRelayProfile {
            api_url,
            kernel_credential: Some("synthetic-kernel-credential".into()),
            ..Default::default()
        };
        for _ in 0..8 {
            let error = crate::runtime::cloud_api_client::get_cloud_kernel_directory(&profile)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("rate_limited"), "{error}");
        }
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_pause_is_per_cloud_and_ends_with_an_answer() {
        let (refusing, _) = cloud(1);
        let (other, other_seen) = cloud(0);
        assert!(matches!(call(&refusing), Err(Error::Status(429, _))));
        assert!(call(&other).is_ok(), "another Cloud is not paused");
        assert_eq!(other_seen.load(Ordering::SeqCst), 1);
        answered(&origin(&refusing).unwrap());
        assert!(call(&refusing).is_ok());
    }

    #[test]
    fn repeated_refusals_back_off_beyond_retry_after() {
        let cloud = "http://127.0.0.1:1".to_string();
        refused(&cloud, Some(Duration::from_secs(2)));
        let first = remaining(&cloud).unwrap();
        refused(&cloud, Some(Duration::from_secs(2)));
        refused(&cloud, Some(Duration::from_secs(2)));
        let third = remaining(&cloud).unwrap();
        assert!(first <= Duration::from_secs(2));
        assert!(third > Duration::from_secs(3), "{third:?}");
        answered(&cloud);
        assert!(remaining(&cloud).is_none());
    }
}
