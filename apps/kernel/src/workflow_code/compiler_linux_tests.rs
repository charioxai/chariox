//! MP-08 / MP-11: runtime discovery remains limited to individual assets.
use super::*;

#[test]
fn linux_compiler_runtime_data_accepts_only_known_asset_names() {
    for name in [
        "/usr/share/nodejs/cjs-module-lexer/lexer.js",
        "/usr/share/nodejs/acorn/dist/acorn.js",
        "/usr/share/nodejs/minimatch/dist/cjs/index.bundle.js",
        "/usr/share/icu/icudt78l.dat",
        "/opt/runtime/icudt78b.dat",
    ] {
        assert_eq!(
            runtime_data_path(name.as_bytes()),
            Some(PathBuf::from(name))
        );
    }
    for name in [
        "/usr/share/nodejs",
        "/etc/hostname",
        "/usr/share/nodejs/private.js",
        "relative/acorn/dist/acorn.js",
        "/opt/../acorn/dist/acorn.js",
        "/opt/icudtxl.dat",
        "/opt/icudt78.dat",
    ] {
        assert!(runtime_data_path(name.as_bytes()).is_none());
    }
}

#[test]
fn linux_compiler_runtime_files_are_regular_files_without_parent_mounts() {
    let node = discover_workflow_code_node_path().unwrap();
    let files = runtime_files(&node).unwrap();
    assert!(!files.is_empty());
    for path in &files {
        assert!(std::fs::metadata(path).unwrap().is_file());
        assert!(!files.contains(path.parent().unwrap()));
    }
}

#[test]
fn linux_compiler_runtime_reader_rejects_invalid_elf_bounds() {
    let worktree = crate::test_support::TestWorktree::new("compiler-elf-bounds");
    let binary = worktree.path().join("runtime");
    let mut header = [0u8; 64];
    header[..6].copy_from_slice(b"\x7fELF\x02\x01");
    header[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    header[54..56].copy_from_slice(&56u16.to_le_bytes());
    header[56..58].copy_from_slice(&1u16.to_le_bytes());
    std::fs::write(&binary, header).unwrap();
    assert!(external_data_files(&binary).is_err());
}

#[test]
fn linux_compiler_runtime_reader_supports_elf_widths_and_byte_orders() {
    let worktree = crate::test_support::TestWorktree::new("compiler-elf-layouts");
    let asset = worktree.path().join("cjs-module-lexer/lexer.js");
    fs::create_dir(asset.parent().unwrap()).unwrap();
    fs::write(&asset, "").unwrap();
    let binary = worktree.path().join("runtime");
    // Read-only and read+execute segments are scanned: outside x86 separate-code
    // layouts, .rodata lives in the executable text segment. Writable ones are not.
    for (flags, scanned) in [(6, false), (5, true), (4, true)] {
        for wide in [false, true] {
            for little in [false, true] {
                let mut bytes = vec![0u8; 256];
                bytes[..4].copy_from_slice(b"\x7fELF");
                bytes[4] = if wide { 2 } else { 1 };
                bytes[5] = if little { 1 } else { 2 };
                let mut write = |at: usize, value: u64, width: usize| {
                    let data = if little {
                        value.to_le_bytes()
                    } else {
                        value.to_be_bytes()
                    };
                    bytes[at..at + width].copy_from_slice(if little {
                        &data[..width]
                    } else {
                        &data[8 - width..]
                    });
                };
                write(if wide { 32 } else { 28 }, 64, if wide { 8 } else { 4 });
                write(if wide { 54 } else { 42 }, if wide { 56 } else { 32 }, 2);
                write(if wide { 56 } else { 44 }, 1, 2);
                write(64, 1, 4);
                write(64 + if wide { 4 } else { 24 }, flags, 4);
                write(64 + if wide { 8 } else { 4 }, 256, if wide { 8 } else { 4 });
                write(
                    64 + if wide { 32 } else { 16 },
                    asset.as_os_str().len() as u64 + 1,
                    if wide { 8 } else { 4 },
                );
                bytes.extend_from_slice(asset.to_str().unwrap().as_bytes());
                bytes.push(0);
                fs::write(&binary, bytes).unwrap();
                let expected = if scanned {
                    BTreeSet::from([asset.clone()])
                } else {
                    BTreeSet::new()
                };
                assert_eq!(external_data_files(&binary).unwrap(), expected);
            }
        }
    }
    fs::remove_file(&asset).unwrap();
    assert!(external_data_files(&binary).is_err());
}
