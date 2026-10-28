use std::{borrow::Cow, convert::Infallible};

use anyhow::{bail, Result};
use wasm_compose::graph::{Component, CompositionGraph, EncodeOptions};
use wasm_encoder::reencode::{Error, Reencode, ReencodeComponent};
use wasmparser::{Validator, WasmFeatures};
use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{decoding::DecodedWasm, ManglingAndAbi};

// Rust std and the pinned preview1 adapter use these for ambient process
// facilities. They are deliberately traps, never linked to a host capability.
const DISABLED_CLI: &[&str] = &[
    "wasi:cli/environment@0.2.0",
    "wasi:cli/exit@0.2.0",
    "wasi:cli/stdin@0.2.0",
    "wasi:cli/terminal-input@0.2.0",
    "wasi:cli/terminal-output@0.2.0",
    "wasi:cli/terminal-stdin@0.2.0",
    "wasi:cli/terminal-stdout@0.2.0",
    "wasi:cli/terminal-stderr@0.2.0",
];

#[derive(Default)]
struct RuntimeNames {
    depth: usize,
}
impl Reencode for RuntimeNames {
    type Error = Infallible;
}
impl ReencodeComponent for RuntimeNames {
    fn push_depth(&mut self) {
        self.depth += 1;
    }
    fn pop_depth(&mut self) {
        self.depth -= 1;
    }
    fn parse_component_import_section(
        &mut self,
        section: &mut wasm_encoder::ComponentImportSection,
        reader: wasmparser::ComponentImportSectionReader<'_>,
    ) -> Result<(), Error<Self::Error>> {
        for import in reader {
            let import = import?;
            let name = import.name.full_name();
            // Only the pinned toolchains' patch level is adapted. The final
            // structural WIT check proves that no newer members/signatures leak.
            let mapped = if self.depth == 0 && name.starts_with("wasi:") {
                name.strip_suffix("@0.2.12")
                    .map(|base| format!("{base}@0.2.0"))
            } else {
                None
            };
            section.import(
                mapped.as_deref().unwrap_or(&name),
                self.component_type_ref(import.ty)?,
            );
        }
        Ok(())
    }
}

pub fn prepare_component(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > chariox_app_wit::MAX_COMPONENT_BYTES {
        bail!("component exceeds the 64 MiB file limit");
    }
    Validator::new_with_features(WasmFeatures::all()).validate_all(bytes)?;
    let mut encoded = wasm_encoder::Component::new();
    RuntimeNames::default().parse_component(&mut encoded, wasmparser::Parser::new(0), bytes)?;
    let bytes = encoded.finish();
    let DecodedWasm::Component(mut resolve, world) = wit_component::decode(&bytes)? else {
        bail!("expected a backend component");
    };
    let disabled: Vec<_> = resolve.worlds[world]
        .imports
        .iter()
        .filter(|(key, _)| DISABLED_CLI.contains(&resolve.name_world_key(key).as_str()))
        .map(|(key, _)| resolve.name_world_key(key))
        .collect();
    let needs_random = resolve.worlds[world].imports.keys().any(|key| {
        matches!(
            resolve.name_world_key(key).as_str(),
            "wasi:random/insecure-seed@0.2.0" | "wasi:random/insecure@0.2.0"
        )
    });
    let result = if disabled.is_empty() && !needs_random {
        bytes
    } else {
        // Parsing elaborates dependencies between exported interfaces (stdin's
        // input-stream, for example). Constructing World directly loses them.
        let exports = disabled
            .iter()
            .map(|name| format!("export {name};"))
            .collect::<String>();
        let package = resolve.push_str(
            "disabled.wit",
            &format!("package chariox:sdk-compat; world disabled {{ {exports} }}"),
        )?;
        let stub_world = resolve.select_world(&[package], Some("disabled"))?;
        let mut core = dummy_module(&resolve, stub_world, ManglingAndAbi::Standard32);
        embed_component_metadata(&mut core, &resolve, stub_world, StringEncoding::UTF8, false)?;
        let stub = ComponentEncoder::default()
            .module(&core)?
            .validate(true)
            .encode()?;
        let mut features = WasmFeatures::all();
        features.set(WasmFeatures::CM_ASYNC, true);
        let mut validator = Validator::new_with_features(features);
        let mut graph = CompositionGraph::new();
        let app = graph.add_component(Component::from_bytes(
            &mut validator,
            "backend",
            Cow::Borrowed(bytes.as_slice()),
        )?)?;
        let disabled = graph.add_component(Component::from_bytes(
            &mut validator,
            "disabled-process",
            Cow::Borrowed(stub.as_slice()),
        )?)?;
        let app_instance = graph.instantiate(app)?;
        let stub_instance = graph.instantiate(disabled)?;
        let app_component = graph
            .get_component(app)
            .ok_or_else(|| anyhow::anyhow!("backend graph missing"))?;
        let stub_component = graph
            .get_component(disabled)
            .ok_or_else(|| anyhow::anyhow!("stub graph missing"))?;
        let edges: Vec<_> = app_component
            .imports()
            .filter_map(|(index, name, _)| {
                stub_component
                    .export_by_name(name)
                    .map(|(export, _, _)| (index, export))
            })
            .collect();
        for (import, export) in edges {
            graph.connect(stub_instance, Some(export), app_instance, import)?;
        }
        if needs_random {
            let random = graph.add_component(Component::from_bytes(
                &mut validator,
                "random-compat",
                include_bytes!("random.wat").as_slice(),
            )?)?;
            let random_instance = graph.instantiate(random)?;
            let edges: Vec<_> = graph
                .get_component(app)
                .unwrap()
                .imports()
                .filter_map(|(index, name, _)| {
                    graph
                        .get_component(random)
                        .unwrap()
                        .export_by_name(name)
                        .map(|(export, _, _)| (index, export))
                })
                .collect();
            for (import, export) in edges {
                graph.connect(random_instance, Some(export), app_instance, import)?;
            }
        }
        graph.encode(EncodeOptions {
            define_components: true,
            export: Some(app_instance),
            validate: true,
        })?
    };
    chariox_app_wit::check_component(&result, chariox_app_wit::APP_WORLD)?;
    Ok(result)
}
