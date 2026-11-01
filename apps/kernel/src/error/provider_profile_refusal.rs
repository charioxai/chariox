//! MP-08 / MP-10 / MP-11: a refusal carries a reason, never provider diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProfileRefusal {
    Unavailable,
    Unverified,
    Unauthenticated,
    UsageExhausted,
}

impl ProviderProfileRefusal {
    pub(crate) fn local_message(self) -> &'static str {
        match self {
            Self::Unavailable => "not registered for this owner on this kernel; select or reconnect an account in Provider Accounts",
            Self::Unverified => "has not been verified; refresh it in Provider Accounts, then retry",
            Self::Unauthenticated => "not authenticated; authenticate or reconnect it in Provider Accounts, then retry",
            Self::UsageExhausted => "usage allowance and credits are exhausted; refresh usage or select another account in Provider Accounts",
        }
    }

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "provider_profile_unavailable",
            Self::Unverified => "provider_profile_unverified",
            Self::Unauthenticated => "provider_profile_unauthenticated",
            Self::UsageExhausted => "provider_profile_usage_exhausted",
        }
    }
}
