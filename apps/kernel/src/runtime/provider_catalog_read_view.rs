//! MP-08 / MP-10 / MP-11: startup summaries share the authorized full catalog cache.
use crate::provider::OpenCodeProviderCatalog;

pub(super) fn catalog_read_view(
    mut catalog: OpenCodeProviderCatalog,
    summary_only: bool,
    focus_provider: Option<&str>,
) -> OpenCodeProviderCatalog {
    if !summary_only && focus_provider.is_none() {
        return catalog;
    }
    for provider in &mut catalog.all {
        let focused = !summary_only
            && focus_provider.is_some_and(|focus| {
                let requested = crate::provider::canonical_provider_family(focus);
                let family =
                    crate::provider::canonical_provider_family(&provider.id).unwrap_or("opencode");
                requested == Some(family) || provider.id == focus
            });
        if !focused {
            let default = catalog.default.get(&provider.id);
            provider.models.retain(|id, _| default == Some(id));
        }
    }
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{OpenCodeProviderInfo, OpenCodeProviderModel};
    use std::collections::BTreeMap;
    #[test]
    fn startup_summary_preserves_providers_defaults_and_lazy_model_reachability() {
        let models = (0..4000)
            .map(|i| {
                (
                    format!("m{i}"),
                    OpenCodeProviderModel {
                        id: format!("m{i}"),
                        name: format!("Model {i}"),
                        status: "active".into(),
                        limit: None,
                        variants: BTreeMap::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let full = OpenCodeProviderCatalog {
            all: ["codex", "opencode-go"]
                .into_iter()
                .map(|id| OpenCodeProviderInfo {
                    id: id.into(),
                    name: id.into(),
                    remote_machine_aliases: vec![],
                    models: models.clone(),
                })
                .collect(),
            default: BTreeMap::from([
                ("codex".into(), "m0".into()),
                ("opencode-go".into(), "m1".into()),
            ]),
            connected: vec!["codex".into(), "opencode-go".into()],
        };
        let summary = catalog_read_view(full.clone(), true, None);
        assert_eq!(summary.default, full.default);
        assert_eq!(summary.connected, full.connected);
        assert!(summary.all.iter().all(|p| p.models.len() == 1));
        assert!(serde_json::to_vec(&summary).unwrap().len() < 1024);
        assert_eq!(catalog_read_view(full.clone(), false, None), full);
        let focused = catalog_read_view(full.clone(), false, Some("codex"));
        assert_eq!(focused.all[0].models, full.all[0].models);
        assert_eq!(focused.all[1].models.len(), 1);
        let opencode = catalog_read_view(full.clone(), false, Some("opencode-go"));
        assert_eq!(opencode.all[1].models, full.all[1].models);
        assert_eq!(opencode.all[0].models.len(), 1);
        let family = catalog_read_view(full.clone(), false, Some("opencode"));
        assert_eq!(
            family.all[1].models, full.all[1].models,
            "an OpenCode account includes its Go service's complete model list"
        );
    }
}
