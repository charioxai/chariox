//! MP-08/MP-10/MP-11: classify official provider failures, never assistant prose.

pub(crate) fn renewal_failed(provider: &str, error: &str) -> bool {
    if !matches!(provider, "codex" | "claude" | "opencode") {
        return false;
    }
    let error = error.to_ascii_lowercase();
    oauth_renewal_evidence(&error)
        || error.contains("provider_authentication_failed")
        || error.contains("claude stopfailure [authentication_failed]")
        || (error.contains("401")
            && error.contains("unauthorized")
            && !error.contains("workspace service"))
}

pub(crate) fn oauth_renewal_evidence(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("refresh_token_reused")
        || error.contains("refresh_token_expired")
        || error.contains("refresh_token_invalidated")
        || ((error.contains("refresh") || error.contains("renew"))
            && (error.contains("401")
                || error.contains("unauthorized")
                || error.contains("invalid_grant")
                || error.contains("revoked")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp08_mp10_mp11_renewal_failure_codes_and_negative_cases() {
        for provider in ["codex", "claude", "opencode"] {
            assert!(renewal_failed(provider, "refresh_token_reused"));
            assert!(renewal_failed(provider, "HTTP 401 Unauthorized"));
            assert!(renewal_failed(
                provider,
                "HTTP 401 Unauthorized after refresh"
            ));
            assert!(renewal_failed(
                provider,
                "OAuth refresh failed: invalid_grant"
            ));
            assert!(!renewal_failed(
                provider,
                "HTTP 401 from a workspace service"
            ));
            assert!(!renewal_failed(provider, "network timeout"));
            assert!(!renewal_failed(provider, "refresh failed: network timeout"));
        }
        assert!(!renewal_failed("dev-stub", "refresh_token_reused"));
        assert!(renewal_failed(
            "claude",
            "Claude StopFailure [authentication_failed]: login required"
        ));
    }
}
