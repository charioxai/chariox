use serde::{Deserialize, Serialize};

use crate::{manifest::valid_app_id, ErrorCode, PackageError, Publisher, Result};

/// The publisher id of the kernel-generated development identity. It contains
/// `_`, which no DNS label or GitHub login can, so no verified namespace can
/// ever equal it. Packages it signs install only on the `dev` channel.
pub const DEV_PUBLISHER_ID: &str = "local_dev";

/// A validated reverse-DNS App ID (`com.example.todo`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AppId(String);

impl AppId {
    pub fn parse(value: &str) -> Result<Self> {
        if !valid_app_id(value) {
            return Err(PackageError::new(
                ErrorCode::InvalidManifest,
                "appId must be a lowercase reverse-DNS identifier",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The namespaces whose verified owner may hold this ID, most specific
    /// first. The last label is the App's own name. `io.github.<login>.<app>`
    /// belongs to that GitHub account only (a `github.io` website proof does
    /// not apply); any other ID belongs to the owner of a domain whose
    /// reversed labels start the ID, with at least two labels (`example.com`).
    /// Local IDs are never checked against these (§4.2).
    pub fn namespaces(&self) -> Vec<Namespace> {
        let labels: Vec<&str> = self.0.split('.').collect();
        if labels[..2] == ["io", "github"] {
            return labels
                .get(2)
                .filter(|_| labels.len() >= 4)
                .map(|login| Namespace::GitHub((*login).to_owned()))
                .into_iter()
                .collect();
        }
        (2..labels.len())
            .rev()
            .map(|count| {
                let domain: Vec<&str> = labels[..count].iter().rev().copied().collect();
                Namespace::Domain(domain.join("."))
            })
            .collect()
    }
}

impl TryFrom<String> for AppId {
    type Error = PackageError;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}

impl From<AppId> for String {
    fn from(value: AppId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum Namespace {
    Domain(String),
    #[serde(rename = "github")]
    GitHub(String),
}

/// Who signed a package, as the kernel must treat it. The development
/// principal is kernel-scoped and is never a publisher credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerPrincipal {
    Dev,
    Publisher {
        publisher_id: String,
        key_id: String,
    },
}

impl SignerPrincipal {
    pub fn of(publisher: &Publisher) -> Self {
        if publisher.id == DEV_PUBLISHER_ID {
            Self::Dev
        } else {
            Self::Publisher {
                publisher_id: publisher.id.clone(),
                key_id: publisher.key_id.clone(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationOutcome {
    /// No publisher holds the ID: registration binds it to this one.
    Bind,
    /// The same publisher already holds it: its data and binding are kept.
    Preserve,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum RegistrationRefusal {
    /// The development identity is not a publisher and cannot register.
    DevIdentity,
    /// A different publisher holds the ID. Never overwritten or reassigned;
    /// resolve by dispute or an explicit new-ID/export/import migration.
    PublisherConflict { bound_publisher_id: String },
}

/// `bound` is the publisher this kernel already bound to the App ID, if any.
/// The copied display name of a manifest never decides.
pub fn registration_outcome(
    bound: Option<&str>,
    incoming: &SignerPrincipal,
) -> std::result::Result<RegistrationOutcome, RegistrationRefusal> {
    let SignerPrincipal::Publisher { publisher_id, .. } = incoming else {
        return Err(RegistrationRefusal::DevIdentity);
    };
    match bound {
        None => Ok(RegistrationOutcome::Bind),
        Some(bound) if bound == publisher_id => Ok(RegistrationOutcome::Preserve),
        Some(bound) => Err(RegistrationRefusal::PublisherConflict {
            bound_publisher_id: bound.to_owned(),
        }),
    }
}
