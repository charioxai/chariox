//! The integer-only, import-free workflow entry profile. Validate every nested
//! core module too: a harmless WIT surface cannot hide a start function or SIMD.
use super::{check_against, invalid, Result};
use wasmparser::{
    CanonicalFunction, CanonicalOption, Parser, Payload, ValType, Validator, WasmFeatures,
};

pub const WORLD: &str = "chariox:workflow-gate/gate@1.0.0";
pub const MAX_COMPONENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_MEMORY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TABLE_ELEMENTS: usize = 4096;
pub const MAX_FUNCTIONS: u32 = 4096;
pub const MAX_INSTANCES: u32 = 16;
pub const WIT: &str = include_str!("../wit-gate/gate.wit");

/// Structural admission only. Compilation and execution must still happen in
/// the contained, verified worker with this profile's fuel and process limits.
pub fn check(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_COMPONENT_BYTES {
        return Err(invalid("gate component exceeds 2 MiB"));
    }
    let features = WasmFeatures::MUTABLE_GLOBAL
        | WasmFeatures::SIGN_EXTENSION
        | WasmFeatures::REFERENCE_TYPES
        | WasmFeatures::GC_TYPES
        | WasmFeatures::MULTI_VALUE
        | WasmFeatures::BULK_MEMORY
        | WasmFeatures::COMPONENT_MODEL;
    Validator::new_with_features(features)
        .validate_all(bytes)
        .map_err(invalid)?;
    let mut memories = 0u32;
    let mut tables = 0u32;
    let mut functions = 0u32;
    let mut instances = 0u32;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(invalid)? {
            Payload::StartSection { .. } | Payload::ComponentStartSection { .. } => {
                return Err(invalid("gate start functions are forbidden"))
            }
            Payload::MemorySection(section) => {
                memories += section.count();
                for memory in section {
                    let memory = memory.map_err(invalid)?;
                    if memory.memory64
                        || memory.shared
                        || memory.initial > 256
                        || memory.maximum.is_some_and(|max| max > 256)
                    {
                        return Err(invalid("gate memory exceeds the 16 MiB profile"));
                    }
                }
            }
            Payload::TableSection(section) => {
                tables += section.count();
                for table in section {
                    let table = table.map_err(invalid)?.ty;
                    if table.table64
                        || table.shared
                        || table.initial > MAX_TABLE_ELEMENTS as u64
                        || table
                            .maximum
                            .is_some_and(|max| max > MAX_TABLE_ELEMENTS as u64)
                        || table.element_type != wasmparser::RefType::FUNCREF
                    {
                        return Err(invalid("gate table exceeds the integer profile"));
                    }
                }
            }
            Payload::TypeSection(section) => {
                for ty in section.into_iter_err_on_gc_types() {
                    let ty = ty.map_err(invalid)?;
                    if ty
                        .params()
                        .iter()
                        .chain(ty.results())
                        .any(|ty| !matches!(ty, ValType::I32 | ValType::I64))
                    {
                        return Err(invalid("gate function types must be integers"));
                    }
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    match import.map_err(invalid)?.ty {
                        wasmparser::TypeRef::Func(_) => functions += 1,
                        // Memory/table imports only wire definitions admitted
                        // elsewhere in this same component; no host linker exists.
                        wasmparser::TypeRef::Memory(_) | wasmparser::TypeRef::Table(_) => {}
                        wasmparser::TypeRef::Global(ty)
                            if matches!(ty.content_type, ValType::I32 | ValType::I64) => {}
                        _ => return Err(invalid("gate core import outside integer profile")),
                    }
                }
            }
            Payload::GlobalSection(section) => {
                for global in section {
                    if !matches!(
                        global.map_err(invalid)?.ty.content_type,
                        ValType::I32 | ValType::I64
                    ) {
                        return Err(invalid("gate global types must be integers"));
                    }
                }
            }
            Payload::FunctionSection(section) => functions += section.count(),
            Payload::InstanceSection(section) => instances += section.count(),
            Payload::ComponentCanonicalSection(section) => {
                for function in section {
                    let options = match function.map_err(invalid)? {
                        CanonicalFunction::Lift { options, .. }
                        | CanonicalFunction::Lower { options, .. } => options,
                        _ => {
                            return Err(invalid(
                                "gate canonical ABI must be synchronous and resource-free",
                            ))
                        }
                    };
                    // Bounded lifting uses borrowed UTF-8 guest strings. Never
                    // transcode a guest-selected length before checking its cap.
                    if options.iter().any(|option| {
                        !matches!(
                            option,
                            CanonicalOption::UTF8
                                | CanonicalOption::Memory(_)
                                | CanonicalOption::Realloc(_)
                                | CanonicalOption::PostReturn(_)
                        )
                    }) {
                        return Err(invalid("gate canonical ABI requires synchronous UTF-8"));
                    }
                }
            }
            _ => {}
        }
        if memories > 1 || tables > 1 || functions > MAX_FUNCTIONS || instances > MAX_INSTANCES {
            return Err(invalid("gate component exceeds structure limits"));
        }
    }
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str("gate.wit", WIT)
        .expect("embedded gate WIT parses");
    let world = resolve
        .select_world(&[package], Some("gate"))
        .expect("embedded gate world");
    check_against(bytes, &resolve, world)
}
