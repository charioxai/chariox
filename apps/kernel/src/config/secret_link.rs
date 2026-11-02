use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

/// Where the owner opens a secret link. An opaque HTTPS origin the kernel only appends
/// `/s/<kernel>/<interaction>` to; a Cloud-enrolled kernel defaults to its Cloud's web app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserSecretLinkConfig {
    pub page_url: Option<String>,
}

impl UserSecretLinkConfig {
    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        match self.page_url.as_deref().map(url::Url::parse) {
            None => Ok(()),
            Some(Ok(url)) if url.scheme() == "https" && url.has_host() => Ok(()),
            Some(_) => Err(DaemonError::InvalidConfig {
                field: "secret_link.page_url",
                message: "the secret page must be an https:// URL",
            }),
        }
    }
}
