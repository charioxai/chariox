use super::*;

const ARM64: u32 = 0x0100_000c;
const X86_64: u32 = 0x0100_0007;

fn command(kind: u32, name: &str) -> Vec<u8> {
    let offset = if kind == LC_RPATH { 12 } else { 24 };
    let mut bytes = [kind, 0, offset].map(u32::to_le_bytes).concat();
    bytes.resize(offset as usize, 0);
    bytes.extend(name.as_bytes());
    bytes.resize((bytes.len() + 8) & !7, 0);
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    bytes
}

fn thin(cpu: u32, commands: &[Vec<u8>]) -> Vec<u8> {
    let body = commands.concat();
    let mut bytes = [
        0xfeed_facf,
        cpu,
        0,
        6,
        commands.len() as u32,
        body.len() as u32,
        0,
        0,
    ]
    .map(u32::to_le_bytes)
    .concat();
    bytes.extend(body);
    bytes
}

fn fat(wide: bool, slices: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = [
        if wide { 0xcafe_babf } else { 0xcafe_babe },
        slices.len() as u32,
    ]
    .map(u32::to_be_bytes)
    .concat();
    for (i, (cpu, slice)) in slices.iter().enumerate() {
        bytes.extend([*cpu, 0].map(u32::to_be_bytes).concat());
        let offset = (i as u64 + 1) * 4096;
        if wide {
            bytes.extend(offset.to_be_bytes());
            bytes.extend((slice.len() as u64).to_be_bytes());
            bytes.extend([12, 0].map(u32::to_be_bytes).concat());
        } else {
            bytes.extend(
                [offset as u32, slice.len() as u32, 12]
                    .map(u32::to_be_bytes)
                    .concat(),
            );
        }
    }
    for (i, (_, slice)) in slices.iter().enumerate() {
        bytes.resize((i + 1) * 4096, 0);
        bytes.extend(slice);
    }
    bytes
}

fn parse(bytes: &[u8], cpu: u32) -> Result<(Vec<String>, Vec<String>), &'static str> {
    let header = bytes.get(..8).ok_or(INVALID)?;
    let table = bytes.get(8..8 + fat_table_size(header)?).ok_or(INVALID)?;
    let mut libraries = Vec::new();
    let mut rpaths = Vec::new();
    for offset in slice_offsets(header, table, cpu)? {
        let offset = usize::try_from(offset).map_err(|_| INVALID)?;
        let header = bytes.get(offset..offset + 32).ok_or(INVALID)?;
        let commands = bytes.get(offset + 32..).ok_or(INVALID)?;
        let (slice_libraries, slice_rpaths) = load_commands(header, commands)?;
        libraries.extend(slice_libraries);
        rpaths.extend(slice_rpaths);
    }
    Ok((libraries, rpaths))
}

#[test]
fn macos_macho_fat_selects_only_kernel_cpu_libraries_and_rpaths() {
    for wide in [false, true] {
        for (host, foreign) in [(ARM64, X86_64), (X86_64, ARM64)] {
            let host_slice = thin(
                host,
                &[
                    command(0xc, "/usr/lib/libSystem.B.dylib"),
                    command(LC_RPATH, "@loader_path/host"),
                ],
            );
            let foreign_slice = thin(
                foreign,
                &[
                    command(0xc, "@rpath/foreign-only-missing.dylib"),
                    command(LC_RPATH, "@loader_path/foreign"),
                ],
            );
            for slices in [
                vec![(foreign, foreign_slice.clone()), (host, host_slice.clone())],
                vec![(host, host_slice.clone()), (foreign, foreign_slice.clone())],
                vec![(host, host_slice.clone())],
            ] {
                let (libraries, rpaths) = parse(&fat(wide, &slices), host).unwrap();
                assert_eq!(
                    libraries,
                    ["/usr/lib/libSystem.B.dylib"],
                    "wide={wide}, host={host:#x}"
                );
                assert_eq!(rpaths, ["@loader_path/host"]);
            }
        }
    }
}

#[test]
fn macos_macho_fat_fails_closed_without_kernel_cpu_slice() {
    for wide in [false, true] {
        for (host, foreign) in [(ARM64, X86_64), (X86_64, ARM64)] {
            let bytes = fat(
                wide,
                &[(foreign, thin(foreign, &[command(0xc, "/foreign.dylib")]))],
            );
            assert!(parse(&bytes, host).is_err(), "wide={wide}, host={host:#x}");
        }
    }
}

#[test]
fn macos_macho_reads_thin_load_commands_and_rejects_truncation() {
    let bytes = thin(
        ARM64,
        &[
            command(0xd, "@rpath/libself.dylib"),
            command(0xc, "/usr/lib/libSystem.B.dylib"),
            command(0x18 | LC_REQ_DYLD, "@loader_path/libweak.dylib"),
            command(LC_RPATH, "@loader_path/../lib"),
        ],
    );
    let (libraries, rpaths) = parse(&bytes, ARM64).unwrap();
    assert_eq!(
        libraries,
        ["/usr/lib/libSystem.B.dylib", "@loader_path/libweak.dylib"]
    );
    assert_eq!(rpaths, ["@loader_path/../lib"]);
    assert!(parse(&bytes[..bytes.len() - 8], ARM64).is_err());
}

#[test]
fn macos_macho_fat_bounds_table_and_preserves_64_bit_offsets() {
    for wide in [false, true] {
        let bytes = fat(wide, &[(ARM64, thin(ARM64, &[]))]);
        let size = fat_table_size(&bytes[..8]).unwrap();
        assert!(slice_offsets(&bytes[..8], &bytes[8..8 + size - 1], ARM64).is_err());
        for count in [0, 17, u32::MAX] {
            let mut header = bytes[..8].to_vec();
            header[4..8].copy_from_slice(&count.to_be_bytes());
            assert!(fat_table_size(&header).is_err());
        }
    }
    let mut bytes = fat(true, &[(ARM64, thin(ARM64, &[]))]);
    let offset = 0x1_0000_1000u64;
    bytes[16..24].copy_from_slice(&offset.to_be_bytes());
    assert_eq!(
        slice_offsets(&bytes[..8], &bytes[8..40], ARM64).unwrap(),
        [offset]
    );
}

#[test]
fn macos_macho_thin_node_selects_its_own_cpu_over_kernel_cpu() {
    // Thin x86_64 Node on an arm64 kernel, and thin arm64 Node on an x86_64 kernel.
    for (node, kernel) in [(X86_64, ARM64), (ARM64, X86_64)] {
        let cpu = runtime_cpu_type(&thin(node, &[])[..8], kernel).unwrap();
        assert_eq!(cpu, node, "node={node:#x}, kernel={kernel:#x}");
        let library = fat(
            false,
            &[
                (kernel, thin(kernel, &[command(0xc, "/kernel-only.dylib")])),
                (node, thin(node, &[command(0xc, "/node-only.dylib")])),
            ],
        );
        assert_eq!(parse(&library, cpu).unwrap().0, ["/node-only.dylib"]);
    }
}

#[test]
fn macos_macho_fat_node_selects_kernel_cpu() {
    for wide in [false, true] {
        for kernel in [ARM64, X86_64] {
            let node = fat(
                wide,
                &[(ARM64, thin(ARM64, &[])), (X86_64, thin(X86_64, &[]))],
            );
            assert_eq!(runtime_cpu_type(&node[..8], kernel), Ok(kernel));
        }
    }
    assert!(runtime_cpu_type(&[0; 8], ARM64).is_err());
}
