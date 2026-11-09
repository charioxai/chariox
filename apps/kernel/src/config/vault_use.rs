//! MP-08 / MP-09 / MP-10 / MP-11: one declared destination per Vault entry.
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultUseBinding {
    pub target: VaultUseTarget,
    #[serde(default)]
    pub value_kind: VaultValueKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultValueKind {
    #[default]
    Secret,
    PaymentCard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultUseTarget {
    Site {
        origin: String,
    },
    CommandEnvironment {
        name: String,
        origin: String,
    },
    CommandStdin {
        origin: String,
    },
    File {
        path: String,
    },
    Totp {
        origin: String,
    },
    WebAuthn {
        origin: String,
        rp_id: String,
    },
    /// A saved payment card: usable at any observed site, each payment approved by the owner.
    AnySite,
}

fn invalid() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "vault_use_binding",
        message: "Vault entries need one exact use: a secure origin, environment name and origin, or absolute file path".into(),
    }
}

/// Shared canonicalization for creation and the observed destination at use.
pub(crate) fn vault_origin(raw: &str) -> Result<String, DaemonError> {
    let url = url::Url::parse(raw.trim()).map_err(|_| invalid())?;
    let host = url.host_str().ok_or_else(invalid)?;
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    Ok(url.origin().ascii_serialization())
}

impl VaultUseTarget {
    pub(crate) fn site(url: &str) -> Result<Self, DaemonError> {
        Ok(Self::Site {
            origin: vault_origin(url)?,
        })
    }

    /// Checks one exact use. Any site is never a requested or agent-declared use: only the
    /// owner's payment card binding names it (`VaultUseBinding::validate`).
    pub(crate) fn validate(&self) -> Result<(), DaemonError> {
        let origin = match self {
            Self::AnySite => return Err(invalid()),
            Self::Site { origin } | Self::CommandStdin { origin } | Self::Totp { origin } => origin,
            Self::CommandEnvironment { name, origin } => {
                let mut chars = name.chars();
                if name.len() > 128
                    || !chars
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return Err(invalid());
                }
                origin
            }
            Self::File { path } => {
                let file = std::path::Path::new(path);
                // The file tool resolves symlinks and its workspace boundary first.
                return if file.is_absolute()
                    && !path.contains('\0')
                    && file.components().all(|c| {
                        matches!(
                            c,
                            std::path::Component::RootDir | std::path::Component::Normal(_)
                        )
                    })
                    && file.components().count() > 1
                    && !path.contains("//")
                    && !path.ends_with('/')
                    && !path.split('/').any(|part| part == "." || part == "..")
                {
                    Ok(())
                } else {
                    Err(invalid())
                };
            }
            Self::WebAuthn { origin, rp_id } => {
                let url = url::Url::parse(origin).map_err(|_| invalid())?;
                let host = url.host_str().ok_or_else(invalid)?;
                if rp_id.is_empty()
                    || rp_id.starts_with('.')
                    || rp_id.ends_with('.')
                    || !rp_id.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                    })
                    || !(host == rp_id || host.ends_with(&format!(".{rp_id}")))
                {
                    return Err(invalid());
                }
                origin
            }
        };
        if vault_origin(origin)? != *origin {
            return Err(invalid());
        }
        Ok(())
    }
}

impl VaultUseBinding {
    /// A payment card is saved for any site; every other entry for one exact use.
    pub(crate) fn validate(&self) -> Result<(), DaemonError> {
        match (&self.target, self.value_kind) {
            (VaultUseTarget::AnySite, VaultValueKind::PaymentCard) => Ok(()),
            (VaultUseTarget::AnySite, VaultValueKind::Secret) => Err(invalid()),
            (target, _) => target.validate(),
        }
    }

    pub(crate) fn require(&self, target: &VaultUseTarget) -> Result<(), DaemonError> {
        self.validate()?;
        target.validate()?;
        let allowed = match &self.target {
            VaultUseTarget::AnySite => matches!(target, VaultUseTarget::Site { .. }),
            bound => bound == target,
        };
        if !allowed {
            return Err(DaemonError::LocalTransport {
                operation: "vault_use_binding",
                message: "this Vault entry was created for a different use; create a separate entry for this destination".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_binding_cannot_change_origin_channel_name_or_path() {
        let targets = [
            VaultUseTarget::site("https://example.com/signup").unwrap(),
            VaultUseTarget::site("https://example.com:444").unwrap(),
            VaultUseTarget::site("https://sub.example.com").unwrap(),
            VaultUseTarget::CommandEnvironment {
                name: "API_KEY".into(),
                origin: "https://example.com".into(),
            },
            VaultUseTarget::CommandEnvironment {
                name: "OTHER_KEY".into(),
                origin: "https://example.com".into(),
            },
            VaultUseTarget::CommandStdin {
                origin: "https://example.com".into(),
            },
            VaultUseTarget::File {
                path: "/work/.env".into(),
            },
            VaultUseTarget::File {
                path: "/work/other/.env".into(),
            },
            VaultUseTarget::Totp {
                origin: "https://example.com".into(),
            },
            VaultUseTarget::WebAuthn {
                origin: "https://example.com".into(),
                rp_id: "example.com".into(),
            },
        ];
        for (i, target) in targets.iter().enumerate() {
            let binding = VaultUseBinding {
                target: target.clone(),
                value_kind: VaultValueKind::Secret,
            };
            for (j, requested) in targets.iter().enumerate() {
                assert_eq!(binding.require(requested).is_ok(), i == j);
            }
        }
    }

    #[test]
    fn a_payment_card_binding_takes_any_site_and_nothing_else() {
        let card = VaultUseBinding {
            target: VaultUseTarget::AnySite,
            value_kind: VaultValueKind::PaymentCard,
        };
        assert!(card.validate().is_ok());
        for site in ["https://shop.example", "https://other.example:8443"] {
            assert!(card.require(&VaultUseTarget::site(site).unwrap()).is_ok());
        }
        for other in [
            VaultUseTarget::AnySite,
            VaultUseTarget::CommandStdin {
                origin: "https://shop.example".into(),
            },
            VaultUseTarget::Totp {
                origin: "https://shop.example".into(),
            },
            VaultUseTarget::File {
                path: "/work/card".into(),
            },
        ] {
            assert!(card.require(&other).is_err());
        }
        assert!(VaultUseTarget::AnySite.validate().is_err());
        assert!(VaultUseBinding {
            target: VaultUseTarget::AnySite,
            value_kind: VaultValueKind::Secret,
        }
        .validate()
        .is_err());
        assert_eq!(
            serde_json::to_value(&card).unwrap(),
            serde_json::json!({"target": {"kind": "any_site"}, "value_kind": "payment_card"})
        );
    }

    #[test]
    fn ambiguous_bindings_are_rejected() {
        for url in [
            "http://example.com",
            "https://u:p@example.com",
            "ftp://example.com",
            "not a url",
        ] {
            assert!(VaultUseTarget::site(url).is_err());
        }
        for path in [
            "relative",
            "/",
            "/work/../other",
            "/work//key",
            "/work/key/",
        ] {
            assert!(VaultUseTarget::File { path: path.into() }
                .validate()
                .is_err());
        }
        assert!(VaultUseTarget::CommandEnvironment {
            name: "BAD=NAME".into(),
            origin: "https://example.com".into()
        }
        .validate()
        .is_err());
        assert!(VaultUseTarget::Totp {
            origin: "https://example.com/path".into()
        }
        .validate()
        .is_err());
    }
}
