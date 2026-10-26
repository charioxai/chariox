//! Save orchestration keeps file lifetime tied to the version-checked pointer.
use crate::{document, error, invalid, read_document_at, AppError};
use serde_json::Value;
use std::path::Path;

pub async fn workbook(
    data: &Path,
    name: &str,
    version: &Value,
    mut get: impl AsyncFnMut() -> Result<Option<(Value, u64)>, AppError>,
    mut write: impl AsyncFnMut(&Value) -> Result<Value, AppError>,
    mut publish: impl AsyncFnMut(Value, Option<u64>) -> Result<u64, AppError>,
) -> Result<u64, AppError> {
    sweep_uploads(data);
    let text = read_document_at(data, name);
    // The editor retains its copy, including on a conflict or invalid upload.
    let _ = std::fs::remove_file(data.join(name));
    let text = text?;
    let wrapper: Value =
        serde_json::from_str(&text).map_err(|_| invalid("file must be a native workbook"))?;
    let book = &wrapper["workbook"];
    document::workbook(book).map_err(invalid)?;
    let version = match version {
        Value::Null => None,
        Value::String(text) => Some(
            text.parse::<u64>()
                .map_err(|_| invalid("version must be a decimal string or null"))?,
        ),
        _ => return Err(invalid("version must be a decimal string or null")),
    };
    // Reject stale views before I/O; publication rechecks after writing.
    let previous = get().await?;
    if previous.as_ref().map(|(_, version)| *version) != version {
        return Err(error(
            "STATE_CONFLICT",
            "workbook changed; export your edits before reopening the saved copy",
        ));
    }
    let pointer = write(book).await?;
    let result = publish(pointer.clone(), version).await;
    if result.is_ok() {
        if let Some((old, _)) = previous {
            crate::snapshot::remove_unless_current(data, &old, &pointer);
        }
    } else if let Ok(current) = get().await {
        // A competing writer may have published identical content.
        let current = current.map(|(value, _)| value).unwrap_or(Value::Null);
        crate::snapshot::remove_unless_current(data, &pointer, &current);
    }
    result
}

pub fn sweep_uploads(data: &Path) {
    let Ok(entries) = std::fs::read_dir(data) else {
        return;
    };
    // Bound maintenance per call and leave recently touched uploads alone.
    for entry in entries.take(256).flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let name = name
            .strip_prefix('.')
            .and_then(|n| n.strip_suffix(".pending"))
            .unwrap_or(name);
        let Some(id) = name
            .strip_prefix("save-")
            .and_then(|n| n.strip_suffix(".univer.json"))
        else {
            continue;
        };
        if id.len() != 36
            || !id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file()
            && metadata
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() >= 24 * 60 * 60)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{cell::RefCell, fs, time::SystemTime};

    struct Store {
        data: std::path::PathBuf,
        state: RefCell<Option<(Value, u64)>>,
    }

    impl Store {
        fn new() -> Self {
            let data = std::env::temp_dir().join(format!(
                "univer-save-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&data).unwrap();
            Self {
                data,
                state: RefCell::new(None),
            }
        }

        fn pointer(&self, book: &Value) -> Value {
            let bytes = book.to_string().into_bytes();
            let hash = format!("{:x}", Sha256::digest(&bytes));
            fs::write(self.data.join(format!("saved-{hash}.json")), &bytes).unwrap();
            json!({"format":"chariox-univer-snapshot-v1","sha256":hash,"bytes":bytes.len()})
        }

        fn save(&self, label: &str, lose: bool, same_winner: bool) -> Result<u64, AppError> {
            let book = json!({"id":"b","name":label,"sheetOrder":["s"],"sheets":{"s":{
                "id":"s","name":"Sheet1","rowCount":10,"columnCount":10,"cellData":{}}}});
            let name = "save-12345678-1234-1234-1234-123456789abc.univer.json";
            fs::write(
                self.data.join(name),
                json!({"format":"chariox-univer-workbook","version":1,"workbook":book}).to_string(),
            )
            .unwrap();
            let version = self.state.borrow().as_ref().map(|(_, v)| v.to_string());
            futures::executor::block_on(workbook(
                &self.data,
                name,
                &json!(version),
                async || Ok(self.state.borrow().clone()),
                async |book| Ok(self.pointer(book)),
                async |pointer, version| {
                    if lose {
                        if same_winner {
                            *self.state.borrow_mut() = Some((pointer, version.unwrap_or(0) + 1));
                        }
                        return Err(error("STATE_CONFLICT", "another writer won"));
                    }
                    let revision = version.unwrap_or(0) + 1;
                    *self.state.borrow_mut() = Some((pointer, revision));
                    Ok(revision)
                },
            ))
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(&self.data)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }

        fn assert_one_saved(&self) {
            let names = self.names();
            assert_eq!(
                names.len(),
                1,
                "only the committed snapshot should remain: {names:?}"
            );
            let state = self.state.borrow();
            let hash = state.as_ref().unwrap().0["sha256"].as_str().unwrap();
            assert_eq!(names[0], format!("saved-{hash}.json"));
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.data).unwrap();
        }
    }

    #[test]
    fn saves_remove_superseded_snapshots_and_consumed_uploads() {
        let store = Store::new();
        store.save("first", false, false).unwrap();
        store.save("second", false, false).unwrap();
        store.assert_one_saved();
        store.save("second", false, false).unwrap();
        store.assert_one_saved();
    }

    #[test]
    fn failed_cas_removes_its_orphan_but_preserves_the_committed_hash() {
        let store = Store::new();
        store.save("first", false, false).unwrap();
        assert_eq!(
            store.save("loser", true, false).unwrap_err().code,
            "STATE_CONFLICT"
        );
        store.assert_one_saved();
        assert_eq!(
            store.save("first", true, true).unwrap_err().code,
            "STATE_CONFLICT"
        );
        store.assert_one_saved();
    }

    #[test]
    fn stale_and_invalid_saves_consume_uploads_without_writing_snapshots() {
        let store = Store::new();
        store.save("first", false, false).unwrap();
        let name = "save-12345678-1234-1234-1234-123456789abc.univer.json";
        let wrapper = json!({"format":"chariox-univer-workbook","version":1,"workbook":{
            "id":"b","name":"stale","sheetOrder":["s"],"sheets":{"s":{
            "id":"s","name":"Sheet1","rowCount":10,"columnCount":10,"cellData":{}}}}});
        for (text, code) in [
            (wrapper.to_string(), "STATE_CONFLICT"),
            ("broken".into(), "INVALID_ARGUMENT"),
        ] {
            fs::write(store.data.join(name), text).unwrap();
            let result = futures::executor::block_on(workbook(
                &store.data,
                name,
                &Value::Null,
                async || Ok(store.state.borrow().clone()),
                async |_| panic!("rejected upload must not write a snapshot"),
                async |_, _| panic!("rejected upload must not publish"),
            ));
            assert_eq!(result.unwrap_err().code, code);
            store.assert_one_saved();
        }
    }

    #[test]
    fn maintenance_removes_only_old_view_uploads_and_partial_uploads() {
        let store = Store::new();
        let old = SystemTime::now() - std::time::Duration::from_secs(25 * 60 * 60);
        let id = "12345678-1234-1234-1234-123456789abc";
        for name in [
            format!("save-{id}.univer.json"),
            format!(".save-{id}.univer.json.pending"),
            "Budget.univer.json".into(),
            "saved-current.json".into(),
        ] {
            let file = fs::File::create(store.data.join(name)).unwrap();
            file.set_times(fs::FileTimes::new().set_modified(old))
                .unwrap();
        }
        let recent = "save-87654321-1234-1234-1234-123456789abc.univer.json";
        fs::write(store.data.join(recent), "active").unwrap();
        sweep_uploads(&store.data);
        assert_eq!(
            store.names(),
            vec!["Budget.univer.json", recent, "saved-current.json"]
        );
    }
}
