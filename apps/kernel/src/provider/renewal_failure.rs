//! MP-08/MP-10/MP-11: classify official provider failures, never assistant prose.

pub(crate) fn renewal_failed(provider: &str, error: &str) -> bool {
    if !matches!(provider, "codex" | "claude" | "opencode") {
        return false;
    }
    let error = error.to_ascii_lowercase();
    if error.contains("workspace service") || error.contains("workspace routing") {
        return false;
    }
    // MP-08/MP-10: OpenCode may omit the OAuth response code. Recovery still
    // requires a renewable profile because this is not strong OAuth evidence.
    (provider == "opencode"
        && error.trim_end().ends_with("token refresh failed")
        && !error.contains("workspace service"))
        || [
            "access token could not be refreshed",
            "authentication token has been invalidated",
            "refresh token was revoked",
            "please log out and sign in again",
            "please try signing in again",
            "not_logged_in",
            "not logged in",
            "login required",
        ]
        .iter()
        .any(|code| error.contains(code))
        || oauth_renewal_evidence(&error)
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
    fn mp08_mp10_mp11_opencode_generic_refresh_failure_requires_profile_gate() {
        assert!(renewal_failed("opencode", "Token refresh failed"));
        assert!(renewal_failed(
            "opencode",
            "provider protocol failure: Token refresh failed"
        ));
        // The receiving kernel must still establish a renewable login profile.
        assert!(!oauth_renewal_evidence("Token refresh failed"));
        for provider in ["codex", "claude", "dev-stub"] {
            assert!(!renewal_failed(provider, "Token refresh failed"));
        }
        assert!(!renewal_failed(
            "opencode",
            "Token refresh failed: network timeout"
        ));
        assert!(!renewal_failed(
            "opencode",
            "workspace service: Token refresh failed"
        ));
    }

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
    #[test]
    fn official_logged_out_variants_share_recovery_and_slice_classification() {
        for provider in ["codex", "opencode", "claude"] {
            for message in [
                "access token could not be refreshed",
                "authentication token has been invalidated",
                "please log out and sign in again",
                "not_logged_in",
            ] {
                assert!(renewal_failed(provider, message));
                assert!(!renewal_failed(
                    provider,
                    &format!("workspace service: {message}")
                ));
            }
        }
    }
}
