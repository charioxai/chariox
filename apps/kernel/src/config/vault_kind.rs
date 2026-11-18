//! The Kind an owner picks for a Vault entry. It decides which uses the entry serves, so a
//! refusal can name the Kind that would serve the use that was asked for.
use super::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialUse, VaultUseTarget,
    VaultValueKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultKind {
    Password,
    ApiKey,
    TwoFactorCode,
    CommandSecret,
    FileSecret,
    PaymentCard,
}

impl VaultKind {
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "Password" => Self::Password,
            "API key" => Self::ApiKey,
            "2FA code" => Self::TwoFactorCode,
            "Command secret" => Self::CommandSecret,
            "File secret" => Self::FileSecret,
            "Payment card" => Self::PaymentCard,
            _ => return None,
        })
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Password => "Password",
            Self::ApiKey => "API key",
            Self::TwoFactorCode => "2FA code",
            Self::CommandSecret => "Command secret",
            Self::FileSecret => "File secret",
            Self::PaymentCard => "Payment card",
        }
    }

    /// Recorded on an entry the owner made, and read back by clients to show its Kind.
    pub(crate) fn created_by_kind(self) -> &'static str {
        match self {
            Self::Password => "owner_password",
            Self::ApiKey => "owner_api_key",
            Self::TwoFactorCode => "owner_totp",
            Self::CommandSecret => "owner_command",
            Self::FileSecret => "owner_file",
            Self::PaymentCard => "owner_payment_card",
        }
    }

    /// What an entry of this Kind does, in the words a refusal shows the person.
    fn effect(self) -> &'static str {
        match self {
            Self::Password => "browser fill",
            Self::ApiKey => "Authorization header for its site",
            Self::TwoFactorCode => "browser 2FA code",
            Self::CommandSecret => "command environment or stdin",
            Self::FileSecret => "workspace file",
            Self::PaymentCard => "payments",
        }
    }

    /// The Kind of an entry, read from the uses the kernel gave it.
    pub(crate) fn of(credential: &UserCredentialConfig) -> Option<Self> {
        let binding = credential.metadata.as_ref()?.use_binding.as_ref()?;
        match (&binding.target, &credential.injection) {
            (VaultUseTarget::AnySite, _) if binding.value_kind == VaultValueKind::PaymentCard => {
                Some(Self::PaymentCard)
            }
            (VaultUseTarget::Site { .. }, UserCredentialInjectionConfig::Browser) => {
                Some(Self::Password)
            }
            (VaultUseTarget::Site { .. }, UserCredentialInjectionConfig::Header { .. })
                if credential.allowed_uses.contains(&UserCredentialUse::Http) =>
            {
                Some(Self::ApiKey)
            }
            (VaultUseTarget::Totp { .. }, UserCredentialInjectionConfig::Totp(_)) => {
                Some(Self::TwoFactorCode)
            }
            (
                VaultUseTarget::CommandEnvironment { .. } | VaultUseTarget::CommandStdin { .. },
                UserCredentialInjectionConfig::Env,
            ) => Some(Self::CommandSecret),
            (VaultUseTarget::File { .. }, UserCredentialInjectionConfig::File) => {
                Some(Self::FileSecret)
            }
            _ => None,
        }
    }

    /// A saved card and a saved 2FA setup key are stored in their own canonical form: their
    /// value fits no other Kind, and no other value fits them.
    pub(crate) fn change_refusal(self, to: Self) -> Option<String> {
        if self.has_own_value() {
            Some(format!(
                "A {} keeps its Kind and Use: its saved value only fits that Kind. Create a new entry for another Kind or use.",
                self.label()
            ))
        } else if to.has_own_value() {
            Some(format!(
                "An entry cannot become a {}: it needs its own kind of value. Create a new entry.",
                to.label()
            ))
        } else {
            None
        }
    }

    fn has_own_value(self) -> bool {
        matches!(self, Self::PaymentCard | Self::TwoFactorCode)
    }

    /// The Kind that serves a use, with that use in the words of `effect`.
    fn serving(requested: UserCredentialUse) -> Option<(Self, &'static str)> {
        match requested {
            UserCredentialUse::Http => Some((Self::ApiKey, "HTTP requests")),
            UserCredentialUse::Browser => Some((Self::Password, "browser fill")),
            UserCredentialUse::Env => Some((Self::CommandSecret, "a command environment or stdin")),
            UserCredentialUse::File => Some((Self::FileSecret, "a workspace file")),
            _ => None,
        }
    }

    /// The refusal for an entry whose Kind does not serve `requested`, naming the Kind the
    /// person can change it to. None when no change of Kind would serve it.
    pub(crate) fn refusal_for(
        credential: &UserCredentialConfig,
        requested: UserCredentialUse,
    ) -> Option<String> {
        let current = Self::of(credential)?;
        let (wanted, word) = Self::serving(requested)?;
        if current == wanted || current.change_refusal(wanted).is_some() {
            return None;
        }
        let id = &credential.id;
        let (saved, effect) = (current.label(), current.effect());
        Some(if Self::made_by_owner(credential) {
            format!(
                "credential `{id}` is saved as {saved} ({effect}); change Kind to {} in the Vault to use it for {word}",
                wanted.label(),
            )
        } else {
            format!(
                "credential `{id}` is saved as {saved} ({effect}) and was not made in the Vault window, so its Kind cannot change; copy it with chariox.vault.copy_for_use into a new {} entry for {word}",
                wanted.label(),
            )
        })
    }

    /// Only an entry the owner made in the Vault window can change its Kind and Use in place.
    pub(crate) fn made_by_owner(credential: &UserCredentialConfig) -> bool {
        credential
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.created_by_kind.as_deref())
            .is_some_and(|kind| kind.starts_with("owner_"))
    }
}
