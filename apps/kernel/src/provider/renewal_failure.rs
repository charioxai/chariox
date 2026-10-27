//! MP-08/MP-10/MP-11: classify official provider failures, never assistant prose.

pub(crate) fn renewal_failed(provider: &str, error: &str) -> bool {
    if !matches!(provider, "codex" | "claude" | "opencode") {
        return false;
    }
    let error = error.to_ascii_lowercase();
    // MP-08/MP-10: OpenCode may omit the OAuth response code. Recovery still
    // requires a renewable profile because this is not strong OAuth evidence.
    (provider == "claude" && claude_api_auth_failure(&error))
        || (provider == "opencode"
            && error.trim_end().ends_with("token refresh failed")
            && !error.contains("workspace service"))
        || oauth_renewal_evidence(&error)
        || error.contains("provider_authentication_failed")
        || error.contains("claude stopfailure [authentication_failed]")
        || (error.contains("401")
            && error.contains("unauthorized")
            && !error.contains("workspace service"))
}

/// A model/organization permission denial does not establish an invalid token.
pub(crate) fn claude_api_auth_failure(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    !error.contains("workspace service")
        && (error.contains("api error: 401")
            || (error.contains("api error: 403")
                && [
                    "authentication_error",
                    "invalid authentication",
                    "invalid credentials",
                    "authentication failed",
                    "not authenticated",
                    "token has expired",
                    "token is invalid",
                    "token revoked",
                ]
                .iter()
                .any(|marker| error.contains(marker))))
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
}

#[cfg(test)]
mod setup_token_tests {
    #[test]
    fn mp08_mp10_mp11_claude_token_failure_enters_official_login_recovery() {
        for error in [
            "API Error: 401 {authentication_error: OAuth token has expired}",
            "API Error: 403 Invalid authentication credentials",
        ] {
            assert!(super::renewal_failed("claude", error), "{error}");
        }
        assert!(!super::renewal_failed(
            "claude",
            "workspace service: API Error: 401"
        ));
        assert!(!super::renewal_failed("claude", "network timeout"));
        assert!(!super::renewal_failed(
            "claude",
            "API Error: 403 permission_error: model access denied"
        ));
    }
}
