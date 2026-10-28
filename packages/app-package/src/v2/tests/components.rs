use std::path::Path;

use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{LiftLowerAbi, ManglingAndAbi, Resolve};

pub(super) fn build(world: &str, forbidden_import: bool) -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app-wit");
    let mut resolve = Resolve::default();
    let (package, _) = resolve.push_dir(root.join("wit")).unwrap();
    let world = match world {
        chariox_app_wit::APP_WORLD => {
            let mut source = std::fs::read_to_string(root.join("tests/fixtures/v1.wit")).unwrap();
            if forbidden_import {
                source = source.replace(
                    "world probe {",
                    "world probe { import forbidden: async func();",
                );
            }
            let package = resolve.push_str("probe.wit", &source).unwrap();
            resolve.select_world(&[package], Some("probe")).unwrap()
        }
        chariox_app_wit::MIGRATION_WORLD => {
            resolve.select_world(&[package], Some("migration")).unwrap()
        }
        _ => panic!("unregistered fixture world {world}"),
    };
    let mut module = dummy_module(
        &resolve,
        world,
        ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback),
    );
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}
