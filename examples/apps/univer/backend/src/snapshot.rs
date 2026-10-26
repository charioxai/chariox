//! Immutable files keep the structured-state CAS small. Publishing the pointer
//! last means cancellation or a losing writer cannot damage a previous save.
use crate::{document, error, write_bytes, AppError};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read};

const FORMAT: &str = "chariox-univer-snapshot-v1";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn filename(hash: &str) -> String {
    format!("saved-{hash}.json")
}

pub fn remove_unless_current(data: &std::path::Path, candidate: &Value, current: &Value) {
    if candidate["format"] != FORMAT || candidate["sha256"] == current["sha256"] {
        return;
    }
    if let Some(hash) = candidate["sha256"].as_str().filter(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        let _ = std::fs::remove_file(data.join(filename(hash)));
    }
}

pub async fn write(book: &Value) -> Result<Value, AppError> {
    let bytes = book.to_string().into_bytes();
    let hash = digest(&bytes);
    let size = bytes.len();
    write_bytes(filename(&hash), bytes).await?;
    Ok(json!({"format":FORMAT,"sha256":hash,"bytes":size}))
}

pub fn read(record: Value) -> Result<Value, AppError> {
    read_with(record, |name| {
        let mut bytes = Vec::new();
        File::open(format!("data/{name}"))
            .map_err(|_| corrupt())?
            .take((document::MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| corrupt())?;
        Ok(bytes)
    })
}

fn read_with(
    record: Value,
    read_file: impl FnOnce(String) -> Result<Vec<u8>, AppError>,
) -> Result<Value, AppError> {
    if record["format"] != FORMAT {
        // Preserve workbooks saved by the original structured-state backend.
        document::workbook(&record).map_err(|e| error("INVALID_STATE", e))?;
        return Ok(record);
    }
    let hash = record["sha256"]
        .as_str()
        .filter(|v| {
            v.len() == 64
                && v.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        .ok_or_else(corrupt)?;
    let size = record["bytes"]
        .as_u64()
        .filter(|n| (1..=document::MAX_BYTES as u64).contains(n))
        .ok_or_else(corrupt)? as usize;
    let bytes = read_file(filename(hash))?;
    if bytes.len() != size || digest(&bytes) != hash {
        return Err(corrupt());
    }
    let book = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
    document::workbook(&book).map_err(|e| error("INVALID_STATE", e))?;
    Ok(book)
}

fn corrupt() -> AppError {
    error(
        "INVALID_STATE",
        "saved workbook file is missing or damaged; open an exported copy",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_pointer_loads_a_large_snapshot_and_rejects_missing_or_corrupt_bytes() {
        let mut book = json!({"id":"b","name":"","sheetOrder":["s"],"sheets":{"s":{
            "id":"s","name":"Sheet1","rowCount":10000,"columnCount":10,"cellData":{}}}});
        book["name"] = json!("x".repeat(
            document::MAX_BYTES
                - 1
                - json!({"format":"chariox-univer-workbook","version":1,"workbook":book})
                    .to_string()
                    .len()
        ));
        let bytes = book.to_string().into_bytes();
        assert_eq!(
            json!({"format":"chariox-univer-workbook","version":1,"workbook":book})
                .to_string()
                .len(),
            document::MAX_BYTES - 1
        );
        let hash = digest(&bytes);
        let pointer = json!({"format":FORMAT,"sha256":hash,"bytes":bytes.len()});
        assert_eq!(
            read_with(pointer.clone(), |name| {
                assert_eq!(name, filename(&hash));
                Ok(bytes)
            })
            .unwrap(),
            book
        );
        assert!(read_with(pointer.clone(), |_| Err(corrupt())).is_err());
        assert!(read_with(pointer.clone(), |_| Ok(vec![b'x'; document::MAX_BYTES - 1])).is_err());
        let mut wrong_hash = pointer;
        wrong_hash["sha256"] = json!("../escape");
        assert!(read_with(wrong_hash, |_| panic!("invalid pointer must not read")).is_err());
        assert_eq!(
            read_with(book.clone(), |_| panic!("legacy workbook needs no file")).unwrap(),
            book
        );
    }
}
