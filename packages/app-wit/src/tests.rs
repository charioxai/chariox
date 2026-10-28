use super::*;
use wasm_encoder::{
    Component, ComponentImportSection, ComponentTypeRef as EncoderTypeRef, CoreTypeSection,
    ModuleType,
};
use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{LiftLowerAbi, ManglingAndAbi};

fn component(resolve: &Resolve, world: WorldId) -> Vec<u8> {
    let mut module = dummy_module(
        resolve,
        world,
        ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback),
    );
    embed_component_metadata(&mut module, resolve, world, StringEncoding::UTF8, false).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

fn fixture(resolve: &mut Resolve, source: &str) -> Vec<u8> {
    let package = resolve.push_str("fixture.wit", source).unwrap();
    let world = resolve.select_world(&[package], None).unwrap();
    component(resolve, world)
}

fn v1() -> Vec<u8> {
    let (mut resolve, _) = contract::resolve();
    fixture(&mut resolve, include_str!("../tests/fixtures/v1.wit"))
}

#[test]
fn accepts_async_v1_with_streams_and_cancellation() {
    check_component(&v1(), APP_WORLD).unwrap();
}

#[test]
fn accepts_the_complete_allowlist_including_wasi_resources() {
    let (resolve, package) = contract::resolve();
    let world = resolve
        .select_world(&[package], Some("app-admission"))
        .unwrap();
    check_component(&component(&resolve, world), APP_WORLD).unwrap();
}

#[test]
fn migration_world_has_only_state_and_its_required_export() {
    let (resolve, package) = contract::resolve();
    let world = resolve.select_world(&[package], Some("migration")).unwrap();
    check_component(&component(&resolve, world), MIGRATION_WORLD).unwrap();
    assert!(matches!(
        check_component(&v1(), MIGRATION_WORLD),
        Err(ComponentError::ImportNotAllowed { .. })
    ));
}

#[test]
fn an_additive_minor_links_old_imports_and_requires_explicit_world_registration() {
    let old = v1();
    let (mut resolve, _) = contract::resolve();
    let package = resolve
        .push_str("v1.1.wit", include_str!("../tests/fixtures/v1.1.wit"))
        .unwrap();
    let world = resolve.select_world(&[package], Some("app")).unwrap();
    check_against(&old, &resolve, world).unwrap();
    check_against(&component(&resolve, world), &resolve, world).unwrap();
    assert_eq!(
        check_component(&old, "chariox:app/app@1.1.0"),
        Err(ComponentError::UnsupportedWorld {
            world: "chariox:app/app@1.1.0".into()
        })
    );
    let new = component(&resolve, world);
    assert!(
        matches!(check_component(&new, APP_WORLD), Err(ComponentError::ImportNotAllowed { import }) if import == "chariox:app/diagnostics@1.1.0")
    );
}

#[test]
fn rejects_a_changed_signature_under_an_allowed_import_name() {
    let source = include_str!("../wit/app.wit")
        .replace("get: async func(key: string)", "get: async func(key: u32)");
    let (mut forged, _) = contract::with_app(&source);
    let bytes = fixture(
        &mut forged,
        include_str!("../tests/fixtures/incompatible.wit"),
    );
    assert!(matches!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::TypeMismatch { .. })
    ));
}

#[test]
fn rejects_changed_export_signatures_and_sync_downgrades() {
    for (before, after) in [
        ("input: json", "input: u32"),
        ("call-tool: async func", "call-tool: func"),
    ] {
        let source = include_str!("../wit/app.wit").replace(before, after);
        let (mut resolve, _) = contract::with_app(&source);
        let bytes = fixture(&mut resolve, include_str!("../tests/fixtures/v1.wit"));
        assert!(
            matches!(
                check_component(&bytes, APP_WORLD),
                Err(ComponentError::TypeMismatch { .. })
            ),
            "{before}"
        );
    }
}

#[test]
fn rejects_unknown_imports_with_the_exact_binary_name() {
    let (mut resolve, _) = contract::resolve();
    let source = include_str!("../tests/fixtures/v1.wit")
        .replace("world probe {", "world probe { import forbidden: func();");
    let bytes = fixture(&mut resolve, &source);
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::ImportNotAllowed {
            import: "forbidden".into()
        })
    );
}

#[test]
fn rejects_socket_imports_and_unlisted_wasi_versions() {
    for import in [
        "wasi:sockets/types@0.3.0",
        "wasi:cli/environment@0.2.0",
        "wasi:cli/exit@0.2.0",
        "wasi:http/incoming-handler@0.2.0",
    ] {
        let (mut resolve, _) = contract::resolve();
        let source = include_str!("../tests/fixtures/v1.wit")
            .replace("world probe {", &format!("world probe {{ import {import};"));
        let bytes = fixture(&mut resolve, &source);
        assert!(
            matches!(check_component(&bytes, APP_WORLD), Err(ComponentError::ImportNotAllowed { import: name }) if name == import),
            "{import}"
        );
    }
    // No 0.2.* wildcard: even a compatible signature needs an explicit entry.
    let (resolve, package) = contract::resolve();
    let world = resolve
        .select_world(&[package], Some("app-admission"))
        .unwrap();
    let mut bytes = component(&resolve, world);
    let from = b"wasi:random/random@0.2.0";
    let at = bytes
        .windows(from.len())
        .position(|part| part == from)
        .unwrap();
    bytes[at + from.len() - 1] = b'1';
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::ImportNotAllowed {
            import: "wasi:random/random@0.2.1".into()
        })
    );
}

#[test]
fn binary_imports_that_wit_decoding_skips_are_still_refused() {
    let mut types = CoreTypeSection::new();
    types.ty().module(&ModuleType::new());
    let mut imports = ComponentImportSection::new();
    imports.import("hidden-core-module", EncoderTypeRef::Module(0));
    let mut bytes = Component::new();
    bytes.section(&types).section(&imports);
    let bytes = bytes.finish();
    Validator::new().validate_all(&bytes).unwrap();
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::ImportNotAllowed {
            import: "hidden-core-module".into()
        })
    );
}

#[test]
fn refuses_extra_or_missing_exports() {
    let (mut resolve, _) = contract::resolve();
    let source = include_str!("../tests/fixtures/v1.wit")
        .replace("world probe {", "world probe { export extra: func();");
    let bytes = fixture(&mut resolve, &source);
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::ExportNotAllowed {
            export: "extra".into()
        })
    );
    let bytes = wat::parse_str("(component)").unwrap();
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::MissingExport {
            export: "chariox:app/handlers@1.0.0".into()
        })
    );
}

#[test]
fn refuses_malformed_core_and_unknown_major_components() {
    for bytes in [b"not wasm".to_vec(), wat::parse_str("(module)").unwrap()] {
        assert!(matches!(
            check_component(&bytes, APP_WORLD),
            Err(ComponentError::InvalidComponent { .. })
        ));
    }
    let mut concatenated = v1();
    concatenated.extend(v1());
    assert!(matches!(
        check_component(&concatenated, APP_WORLD),
        Err(ComponentError::InvalidComponent { .. })
    ));
    assert_eq!(
        check_component(&v1(), "chariox:app/app@2.0.0"),
        Err(ComponentError::UnsupportedWorld {
            world: "chariox:app/app@2.0.0".into()
        })
    );
}

#[test]
fn rejects_unlisted_members_inside_allowed_interfaces() {
    for (before, after, expected) in [
        (
            "interface state {",
            "interface state { exec: async func(command: string);",
            ComponentError::ImportNotAllowed {
                import: "chariox:app/state@1.0.0#exec".into(),
            },
        ),
        (
            "interface handlers {",
            "interface handlers { extra: async func();",
            ComponentError::ExportNotAllowed {
                export: "chariox:app/handlers@1.0.0#extra".into(),
            },
        ),
    ] {
        let source = include_str!("../wit/app.wit").replace(before, after);
        let (mut resolve, _) = contract::with_app(&source);
        let bytes = fixture(&mut resolve, include_str!("../tests/fixtures/v1.wit"));
        assert_eq!(check_component(&bytes, APP_WORLD), Err(expected));
    }
}

#[test]
fn admission_surface_is_frozen() {
    let (resolve, package) = contract::resolve();
    let mut lines = Vec::new();
    for name in ["app-admission", "app-migrations-admission"] {
        let world = resolve.select_world(&[package], Some(name)).unwrap();
        for (direction, items) in [
            ("import", &resolve.worlds[world].imports),
            ("export", &resolve.worlds[world].exports),
        ] {
            for key in items.keys() {
                lines.push(format!(
                    "{name} {direction} {}",
                    resolve.name_world_key(key)
                ));
            }
        }
    }
    lines.sort();
    let actual = lines.join("\n") + "\n";
    assert_eq!(actual, include_str!("../tests/fixtures/allowlist.txt"));
}

#[test]
fn unknown_export_member_has_typed_refusal() {
    let source = include_str!("../wit/app.wit").replace(
        "interface handlers {",
        "interface handlers { extra: async func();",
    );
    let (mut resolve, _) = contract::with_app(&source);
    let bytes = fixture(&mut resolve, include_str!("../tests/fixtures/v1.wit"));
    assert_eq!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::ExportNotAllowed {
            export: "chariox:app/handlers@1.0.0#extra".into()
        })
    );
}

#[test]
fn sdk_directory_layout_resolves_the_same_contract() {
    let (embedded, embedded_package) = contract::resolve();
    let mut sdk = Resolve::default();
    let (sdk_package, _) = sdk
        .push_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wit"))
        .unwrap();
    for name in [
        "app",
        "migration",
        "app-admission",
        "app-migrations-admission",
    ] {
        let sdk_world = sdk.select_world(&[sdk_package], Some(name)).unwrap();
        let embedded_world = embedded
            .select_world(&[embedded_package], Some(name))
            .unwrap();
        let names = |resolve: &Resolve, world: WorldId| {
            let mut imports: Vec<_> = resolve.worlds[world]
                .imports
                .keys()
                .map(|key| resolve.name_world_key(key))
                .collect();
            imports.sort();
            imports
        };
        assert_eq!(names(&sdk, sdk_world), names(&embedded, embedded_world));
    }
}

#[test]
fn duplicate_allowed_import_names_are_invalid_binary() {
    let bytes = wat::parse_str(
        r#"(component
        (type $i (instance))
        (import "chariox:app/state@1.0.0" (instance (type $i)))
        (import "chariox:app/state@1.0.0" (instance (type $i)))
    )"#,
    )
    .unwrap();
    assert!(matches!(
        check_component(&bytes, APP_WORLD),
        Err(ComponentError::InvalidComponent { .. })
    ));
}
