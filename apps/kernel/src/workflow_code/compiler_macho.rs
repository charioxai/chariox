//! MP-08 / MP-11: bounded, platform-independent Mach-O parsing for compiler isolation.
const INVALID: &str = "compiler runtime is not a supported Mach-O binary";

const LC_REQ_DYLD: u32 = 0x8000_0000;
const LC_RPATH: u32 = 0x1c | LC_REQ_DYLD;
// LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB, LC_LAZY_LOAD_DYLIB, LC_LOAD_UPWARD_DYLIB.
const LC_LOAD_DYLIBS: [u32; 5] = [
    0xc,
    0x18 | LC_REQ_DYLD,
    0x1f | LC_REQ_DYLD,
    0x20,
    0x23 | LC_REQ_DYLD,
];

fn word(bytes: &[u8], at: usize, big_endian: bool) -> Option<u32> {
    let bytes = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(if big_endian {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    })
}

pub(super) fn fat_table_size(header: &[u8]) -> Result<usize, &'static str> {
    match word(header, 0, true) {
        Some(magic @ (0xcafe_babe | 0xcafe_babf)) => {
            let count = word(header, 4, true).ok_or(INVALID)? as usize;
            if count == 0 || count > 16 {
                return Err(INVALID);
            }
            Ok(count * if magic == 0xcafe_babf { 32 } else { 20 })
        }
        _ => Ok(0),
    }
}

/// The architecture Node runs as: a thin binary's own CPU type; a universal
/// binary launches as the kernel's (including under Rosetta).
pub(super) fn runtime_cpu_type(header: &[u8], kernel_cpu_type: u32) -> Result<u32, &'static str> {
    if fat_table_size(header)? > 0 {
        return Ok(kernel_cpu_type);
    }
    if word(header, 0, false) != Some(0xfeed_facf) {
        return Err(INVALID);
    }
    word(header, 4, false).ok_or(INVALID)
}

pub(super) fn slice_offsets(
    header: &[u8],
    table: &[u8],
    cpu_type: u32,
) -> Result<Vec<u64>, &'static str> {
    let size = fat_table_size(header)?;
    if size == 0 {
        return Ok(vec![0]);
    }
    let entry_size = if word(header, 0, true) == Some(0xcafe_babf) {
        32
    } else {
        20
    };
    let table = table.get(..size).ok_or(INVALID)?;
    // Select by Node's runtime architecture. Foreign slices must not
    // contribute library or search-path reads.
    let slices = table
        .chunks_exact(entry_size)
        .filter(|entry| word(entry, 0, true) == Some(cpu_type))
        .map(|entry| match entry_size {
            32 => Ok(u64::from(word(entry, 8, true).ok_or(INVALID)?) << 32
                | u64::from(word(entry, 12, true).ok_or(INVALID)?)),
            _ => word(entry, 8, true).map(u64::from).ok_or(INVALID),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if slices.is_empty() {
        return Err("compiler runtime has no Mach-O slice for the Node architecture");
    }
    Ok(slices)
}

pub(super) fn command_size(header: &[u8]) -> Result<usize, &'static str> {
    if word(header, 0, false) != Some(0xfeed_facf) {
        return Err(INVALID);
    }
    let size = word(header, 20, false).ok_or(INVALID)? as usize;
    if size > 1024 * 1024 {
        return Err(INVALID);
    }
    Ok(size)
}

pub(super) fn load_commands(
    header: &[u8],
    commands: &[u8],
) -> Result<(Vec<String>, Vec<String>), &'static str> {
    let invalid = || INVALID;
    let count = word(header, 16, false).ok_or_else(invalid)?;
    let commands = commands.get(..command_size(header)?).ok_or(INVALID)?;
    let (mut libraries, mut rpaths) = (Vec::new(), Vec::new());
    let mut at = 0;
    for _ in 0..count {
        let command = word(commands, at, false).ok_or_else(invalid)?;
        let length = word(commands, at + 4, false).ok_or_else(invalid)? as usize;
        let body = commands
            .get(at..at + length)
            .filter(|_| length >= 8)
            .ok_or_else(invalid)?;
        let names = if LC_LOAD_DYLIBS.contains(&command) {
            &mut libraries
        } else if command == LC_RPATH {
            &mut rpaths
        } else {
            at += length;
            continue;
        };
        // Both commands store their name as an lc_str offset after the header.
        let name = word(body, 8, false)
            .and_then(|start| body.get(start as usize..))
            .and_then(|name| name.split(|byte| *byte == 0).next())
            .and_then(|name| std::str::from_utf8(name).ok())
            .ok_or_else(invalid)?;
        names.push(name.to_string());
        at += length;
    }
    Ok((libraries, rpaths))
}

#[cfg(test)]
#[path = "compiler_macho_tests.rs"]
mod tests;
