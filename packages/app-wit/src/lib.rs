//! The kernel's offline WIT admission boundary. No App code is instantiated.

mod contract;

use std::{collections::BTreeSet, fmt};

use wasmparser::{
    ComponentExternalKind, ComponentTypeRef, Encoding, Parser, Payload, Validator, WasmFeatures,
};
use wit_parser::{Resolve, WorldId};

pub const APP_WORLD: &str = "chariox:app/app@1.0.0";
pub const MIGRATION_WORLD: &str = "chariox:app/migration@1.0.0";
pub const SUPPORTED_WORLDS: &[&str] = &[APP_WORLD, MIGRATION_WORLD];
pub const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentError {
    UnsupportedWorld { world: String },
    ImportNotAllowed { import: String },
    ExportNotAllowed { export: String },
    MissingExport { export: String },
    InvalidComponent { reason: String },
    TypeMismatch { reason: String },
}

impl fmt::Display for ComponentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedWorld { world } => write!(
                f,
                "unsupported App world `{world}`; rebuild for a supported world"
            ),
            Self::ImportNotAllowed { import } => write!(
                f,
                "App import `{import}` is not allowed; remove it or use the brokered App API"
            ),
            Self::ExportNotAllowed { export } => {
                write!(f, "App export `{export}` is not allowed in this world")
            }
            Self::MissingExport { export } => {
                write!(f, "App component is missing required export `{export}`")
            }
            Self::InvalidComponent { reason } => write!(f, "invalid App component: {reason}"),
            Self::TypeMismatch { reason } => {
                write!(f, "App component does not match its WIT contract: {reason}")
            }
        }
    }
}

impl std::error::Error for ComponentError {}

type Result<T> = std::result::Result<T, ComponentError>;

/// Shared by local install, worker preflight and Library compatibility queries.
/// This checks WIT only: callers must also check trust, resources, kernel
/// features, worker availability and platform containment before activation.
pub fn check_component(bytes: &[u8], world: &str) -> Result<()> {
    let admission = match world {
        APP_WORLD => "app-admission",
        MIGRATION_WORLD => "app-migrations-admission",
        _ => {
            return Err(ComponentError::UnsupportedWorld {
                world: world.into(),
            })
        }
    };
    let (resolve, package) = contract::resolve();
    let world = resolve
        .select_world(&[package], Some(admission))
        .expect("embedded admission world");
    check_against(bytes, &resolve, world)
}

fn invalid(reason: impl fmt::Display) -> ComponentError {
    ComponentError::InvalidComponent {
        reason: reason.to_string(),
    }
}

fn check_against(bytes: &[u8], resolve: &Resolve, world: WorldId) -> Result<()> {
    if bytes.len() > MAX_COMPONENT_BYTES {
        return Err(invalid("component exceeds the 64 MiB file limit"));
    }
    let contract = &resolve.worlds[world];
    let imports: BTreeSet<_> = contract
        .imports
        .keys()
        .map(|key| resolve.name_world_key(key))
        .collect();
    let mut exports: BTreeSet<_> = contract
        .exports
        .keys()
        .map(|key| resolve.name_world_key(key))
        .collect();
    let mut depth = 0;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(invalid)? {
            Payload::Version { encoding, .. } if depth == 0 => {
                if encoding != Encoding::Component {
                    return Err(invalid("expected a component, not a core module"));
                }
            }
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) if depth > 0 => depth -= 1,
            Payload::ComponentImportSection(section) if depth == 0 => {
                for import in section {
                    let import = import.map_err(invalid)?;
                    // Inspect binary imports before WIT decoding, which can omit
                    // core module/component/value imports entirely.
                    if !imports.contains(import.name.name)
                        || !matches!(import.ty, ComponentTypeRef::Instance(_))
                    {
                        return Err(ComponentError::ImportNotAllowed {
                            import: import.name.name.into(),
                        });
                    }
                }
            }
            Payload::ComponentExportSection(section) if depth == 0 => {
                for export in section {
                    let export = export.map_err(invalid)?;
                    if !exports.remove(export.name.name)
                        || export.kind != ComponentExternalKind::Instance
                    {
                        return Err(ComponentError::ExportNotAllowed {
                            export: export.name.name.into(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Validator::new_with_features(WasmFeatures::default() | WasmFeatures::CM_FORWARD)
        .validate_all(bytes)
        .map_err(invalid)?;
    if let Some(export) = exports.into_iter().next() {
        return Err(ComponentError::MissingExport { export });
    }
    check_members(bytes, resolve, world)?;
    // Canonical names permit structurally compatible older imports, but only
    // after the exact versioned allowlist above has admitted every name.
    wit_component::targets(resolve, world, bytes, true).map_err(|error| {
        ComponentError::TypeMismatch {
            reason: format!("{error:#}"),
        }
    })
}

#[cfg(test)]
mod tests;

fn check_members(bytes: &[u8], expected: &Resolve, world: WorldId) -> Result<()> {
    let wit_parser::decoding::DecodedWasm::Component(actual, actual_world) =
        wit_parser::decoding::decode(bytes).map_err(invalid)?
    else {
        return Err(invalid(
            "expected executable component, not an encoded WIT package",
        ));
    };
    for (export, items, allowed) in [
        (
            false,
            &actual.worlds[actual_world].imports,
            &expected.worlds[world].imports,
        ),
        (
            true,
            &actual.worlds[actual_world].exports,
            &expected.worlds[world].exports,
        ),
    ] {
        for (key, item) in items {
            let wit_parser::WorldItem::Interface { id, .. } = item else {
                return Err(invalid("non-interface item after binary admission"));
            };
            let name = actual.name_world_key(key);
            let Some((
                _,
                wit_parser::WorldItem::Interface {
                    id: expected_id, ..
                },
            )) = allowed
                .iter()
                .find(|(key, _)| expected.name_world_key(key) == name)
            else {
                return Err(invalid(format!("interface `{name}` was not admitted")));
            };
            let interface = &actual.interfaces[*id];
            let expected_interface = &expected.interfaces[*expected_id];
            let unexpected_function = interface
                .functions
                .keys()
                .find(|name| !expected_interface.functions.contains_key(*name));
            let unexpected_type = interface
                .types
                .keys()
                .find(|name| !expected_interface.types.contains_key(*name));
            if let Some(member) = unexpected_function.or(unexpected_type) {
                let member = format!("{name}#{member}");
                return Err(if export {
                    ComponentError::ExportNotAllowed { export: member }
                } else {
                    ComponentError::ImportNotAllowed { import: member }
                });
            }
        }
    }
    Ok(())
}
