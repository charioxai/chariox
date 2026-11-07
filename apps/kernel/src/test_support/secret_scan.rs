//! MP-10 / MP-11: finds a secret in everything a run left on disk, in the forms a log line, a
//! serializer or a debug dump would leave it: plain, hex, and base64 at any byte alignment.
use base64::Engine;
use std::path::{Path, PathBuf};

/// A base64 text that does not depend on the bytes around the secret.
fn base64_needle(secret: &[u8], alignment: usize, url_safe: bool) -> Vec<u8> {
    let mut padded = vec![0u8; alignment];
    padded.extend_from_slice(secret);
    let encoded = if url_safe {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&padded)
    } else {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(&padded)
    };
    let leading = (alignment * 8).div_ceil(6);
    encoded.as_bytes()[leading..encoded.len() - 2].to_vec()
}

fn needles(secret: &str) -> Vec<(&'static str, Vec<u8>)> {
    let bytes = secret.as_bytes();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut needles = vec![
        ("plain", bytes.to_vec()),
        ("hex", hex.to_lowercase().into_bytes()),
        ("HEX", hex.to_uppercase().into_bytes()),
    ];
    for alignment in 0..3 {
        needles.push(("base64", base64_needle(bytes, alignment, false)));
        needles.push(("base64url", base64_needle(bytes, alignment, true)));
    }
    needles
}

/// Every file under `root` that holds `secret` in any of its forms, with the form found.
pub(crate) fn files_holding_secret(root: &Path, secret: &str) -> Vec<(PathBuf, &'static str)> {
    let needles = needles(secret);
    let mut hits = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file() {
                let Ok(content) = std::fs::read(&path) else {
                    continue;
                };
                for (form, needle) in &needles {
                    if content.windows(needle.len()).any(|window| window == needle) {
                        hits.push((path.clone(), *form));
                        break;
                    }
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_finds_a_secret_in_every_form_and_alignment() {
        const SECRET: &str = "sk-ant-oat01-FAKESCANPROOF-0123456789abcdef";
        let root =
            std::env::temp_dir().join(format!("chariox-secret-scan-{}", rand::random::<u64>()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let encoded = |prefix: &str| {
            base64::engine::general_purpose::STANDARD
                .encode(format!("{prefix}{SECRET}-and-more").as_bytes())
        };
        std::fs::write(root.join("plain.log"), format!("token={SECRET}")).unwrap();
        for (index, prefix) in ["", "a", "ab"].iter().enumerate() {
            std::fs::write(
                root.join("nested").join(format!("b64-{index}.json")),
                format!("{{\"v\":\"{}\"}}", encoded(prefix)),
            )
            .unwrap();
        }
        let hex: String = SECRET.bytes().map(|byte| format!("{byte:02x}")).collect();
        std::fs::write(root.join("hex.txt"), hex).unwrap();
        std::fs::write(root.join("clean.txt"), "nothing to see").unwrap();

        let hits = files_holding_secret(&root, SECRET);
        let mut names: Vec<_> = hits
            .iter()
            .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "b64-0.json",
                "b64-1.json",
                "b64-2.json",
                "hex.txt",
                "plain.log"
            ]
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
