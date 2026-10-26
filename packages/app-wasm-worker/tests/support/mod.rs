use std::convert::Infallible;
use wasm_encoder::reencode::{self, Reencode};
use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{LiftLowerAbi, ManglingAndAbi, Resolve};

/// The upstream dummy generator imports unused forward intrinsics which
/// Wasmtime 50 does not implement. Remove only these unused fixture imports;
/// production component bytes always go through admission unchanged.
#[derive(Default)]
struct WithoutForward {
    removed: Vec<u32>,
}

impl Reencode for WithoutForward {
    type Error = Infallible;

    fn function_index(&mut self, index: u32) -> Result<u32, reencode::Error<Self::Error>> {
        assert!(!self.removed.contains(&index), "fixture uses a forward intrinsic");
        Ok(index - self.removed.iter().filter(|&&removed| removed < index).count() as u32)
    }

    fn parse_import_section(
        &mut self,
        output: &mut wasm_encoder::ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        let mut function = 0;
        for import in section.into_imports() {
            let import = import?;
            let is_function = matches!(import.ty, wasmparser::TypeRef::Func(_));
            if import.name.contains("[stream-forward-") || import.name.contains("[future-forward-") {
                assert!(is_function);
                self.removed.push(function);
            } else {
                self.parse_import(output, import)?;
            }
            function += u32::from(is_function);
        }
        Ok(())
    }
}

pub fn vector(source: &str, without_forward: bool) -> Vec<u8> {
    let mut resolve = Resolve::default();
    resolve.push_dir("../app-wit/wit").unwrap();
    let package = resolve.push_str("fixture.wit", source).unwrap();
    let world = resolve.select_world(&[package], None).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback));
    if without_forward {
        let mut output = wasm_encoder::Module::new();
        WithoutForward::default()
            .parse_core_module(&mut output, wasmparser::Parser::new(0), &module)
            .unwrap();
        module = output.finish();
    }
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false).unwrap();
    ComponentEncoder::default().module(&module).unwrap().validate(true).encode().unwrap()
}
