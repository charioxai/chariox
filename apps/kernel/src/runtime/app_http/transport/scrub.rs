//! What an account's answer may carry back to the App. The kernel added the
//! account's `Authorization` value to the request, so the answer is filtered:
//! a server that echoes the request (a debugging endpoint, an error page)
//! cannot hand the value to the App.
use zeroize::Zeroizing;

/// The whole answer, buffered (so a value cannot straddle two chunks).
pub(in crate::runtime::app_http) const ANSWER_BYTES: u64 = 1024 * 1024;

const REDACTED: &[u8] = b"[redacted]";

/// Response headers an account's call hands to the App.
pub(super) fn answer_header(name: &str) -> bool {
    name.starts_with("x-ratelimit-")
        || matches!(
            name,
            "content-type"
                | "etag"
                | "last-modified"
                | "link"
                | "location"
                | "retry-after"
                | "x-poll-interval"
                | "x-github-request-id"
        )
}

pub(super) struct Scrub {
    needles: Vec<Zeroizing<Vec<u8>>>,
}

impl Scrub {
    /// The header value and its token, raw and as a URL or JSON string would
    /// carry them, longest first so a needle inside another is removed whole.
    pub(super) fn of(authorization: &str) -> Self {
        let token = authorization.rsplit(' ').next().unwrap_or(authorization);
        let mut needles = Vec::new();
        for value in [authorization, token] {
            for text in [
                value.to_owned(),
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>(),
                serde_json::to_string(value)
                    .map(|json| json[1..json.len() - 1].to_owned())
                    .unwrap_or_default(),
            ] {
                if !text.is_empty() && !needles.iter().any(|known: &Zeroizing<Vec<u8>>| ***known == *text.as_bytes()) {
                    needles.push(Zeroizing::new(text.into_bytes()));
                }
            }
        }
        needles.sort_by_key(|needle| std::cmp::Reverse(needle.len()));
        Self { needles }
    }

    pub(super) fn bytes(&self, input: &[u8]) -> Vec<u8> {
        let mut output = input.to_vec();
        for needle in &self.needles {
            let mut next = Vec::with_capacity(output.len());
            let mut at = 0;
            while let Some(found) = find(&output[at..], needle) {
                next.extend_from_slice(&output[at..at + found]);
                next.extend_from_slice(REDACTED);
                at += found + needle.len();
            }
            next.extend_from_slice(&output[at..]);
            output = next;
        }
        output
    }

    pub(super) fn text(&self, input: &str) -> String {
        self.bytes(input.as_bytes())
            .into_iter()
            .map(char::from)
            .collect()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_removed_raw_encoded_and_inside_another_text() {
        let scrub = Scrub::of("Bearer ghp_SENTINEL+/x\"y");
        let echoed = br#"{"authorization":"Bearer ghp_SENTINEL+/x\"y","t":"ghp_SENTINEL+/x\"y","q":"ghp_SENTINEL%2B%2Fx%22y"}"#;
        let clean = String::from_utf8(scrub.bytes(echoed)).unwrap();
        assert!(!clean.contains("SENTINEL"), "{clean}");
        assert_eq!(scrub.bytes(b"nothing here"), b"nothing here");
        assert_eq!(scrub.text("Bearer ghp_SENTINEL+/x\"y"), "[redacted]");
    }

    #[test]
    fn only_rate_and_cache_headers_pass() {
        for name in ["etag", "link", "x-ratelimit-remaining", "x-poll-interval", "content-type"] {
            assert!(answer_header(name), "{name}");
        }
        for name in ["set-cookie", "authorization", "www-authenticate", "x-oauth-scopes", "server"] {
            assert!(!answer_header(name), "{name}");
        }
    }
}
