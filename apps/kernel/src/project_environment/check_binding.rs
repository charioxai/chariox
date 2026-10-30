//! MP-08 / MP-10 / MP-11: public binding values never leave the owned reader.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroizing;
#[derive(Serialize, Deserialize)]
pub(super) struct PublicBinding {
    pub present: bool,
    pub needs_vault: bool,
    pub digest: Option<String>,
}
pub(super) fn summarize(value: Option<Zeroizing<String>>) -> PublicBinding {
    let value = value.filter(|v| !v.is_empty() && !v.contains('\0'));
    let needs_vault = value
        .as_ref()
        .is_some_and(|v| super::detect_index::credential_content(v));
    PublicBinding {
        present: value.is_some(),
        needs_vault,
        digest: value
            .as_ref()
            .filter(|_| !needs_vault)
            .map(|v| super::aggregate_projection::metadata_digest(&**v)),
    }
}
pub(super) fn inspect(root: Option<&Path>, locator: &ProjectEnvironmentLocator) -> PublicBinding {
    // Defense in depth before any read, including direct self-exec admission.
    if project_environment_public_source_protected(locator) {
        return PublicBinding {
            present: false,
            needs_vault: true,
            digest: None,
        };
    }
    let value = match locator {
        ProjectEnvironmentLocator::EnvFile { path, key } => root
            .and_then(|root| super::resolver::read_environment_file(root, path).ok())
            .and_then(|text| {
                let mut values = text.lines().filter_map(|line| {
                    parse_environment_assignment(line, key)
                        .transpose()
                        .map(|v| v.map(Zeroizing::new))
                });
                let value = values.next()?.ok()?;
                values.next().is_none().then_some(value)
            }),
        ProjectEnvironmentLocator::ConfigFile { path } => {
            root.and_then(|root| super::resolver::read_environment_file(root, path).ok())
        }
        _ => None,
    };
    summarize(value)
}
