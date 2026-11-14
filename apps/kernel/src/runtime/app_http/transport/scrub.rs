//! What an account's answer may carry back to the App. The kernel added the
//! account's `Authorization` value to the request, so the answer is filtered:
//! a server that echoes the request (a debugging endpoint, an error page)
//! cannot hand the value to the App.
use crate::secret::HttpSecretBoundary;

/// The whole answer, buffered (so a value cannot straddle two chunks).
pub(in crate::runtime::app_http) const ANSWER_BYTES: u64 = 1024 * 1024;

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

/// The header value and its token (`Bearer <token>`), in every encoding the
/// boundary knows.
pub(super) fn boundary(authorization: &str) -> HttpSecretBoundary {
    let mut boundary = HttpSecretBoundary::new(authorization);
    boundary.add(authorization.rsplit(' ').next().unwrap_or(authorization));
    boundary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_removed_raw_encoded_and_inside_another_text() {
        let scrub = boundary("Bearer ghp_SENTINEL+/x\"y");
        let echoed = r#"{"authorization":"Bearer ghp_SENTINEL+/x\"y","t":"ghp_SENTINEL+/x\"y","q":"ghp_SENTINEL%2B%2Fx%22y","b":"Z2hwX1NFTlRJTkVMKy94Ink="}"#;
        let clean = scrub.scrub(echoed);
        assert!(!clean.contains("SENTINEL") && !clean.contains("Z2hwX1NFTlRJTkVM"), "{clean}");
        assert_eq!(scrub.scrub("nothing here"), "nothing here");
        assert_eq!(scrub.scrub("Bearer ghp_SENTINEL+/x\"y"), "[redacted]");
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
