//! What moves between the person's computer and the documents: imports the person chooses,
//! exports they are offered, and snapshots the kernel keeps outside the App's data. The App never
//! sees a host path; the kernel's file broker holds the choice and the bytes.

use crate::{
    index,
    model::{self, Kind, NewDoc},
    store,
};
use chariox_app_sdk::{app_types::AppError, error, files, host};
use serde_json::{json, Value};
use std::fs;

const IMPORTS: [(&str, Kind); 5] = [
    (".md", Kind::Markdown),
    (".markdown", Kind::Markdown),
    (".txt", Kind::Markdown),
    (".html", Kind::Html),
    (".htm", Kind::Html),
];

pub fn suffix(name: &str) -> &str {
    name.rfind('.').map_or("", |at| &name[at..])
}

/// A document titled after the file it came from, in the format its suffix names.
pub fn title_and_kind(name: &str) -> (String, Kind) {
    let extension = suffix(name).to_lowercase();
    let kind = IMPORTS
        .iter()
        .find(|(suffix, _)| *suffix == extension)
        .map_or(Kind::Markdown, |(_, kind)| *kind);
    let stem = &name[..name.len() - extension.len()];
    let title: String = if stem.is_empty() { name } else { stem }
        .chars()
        .take(200)
        .collect();
    (title, kind)
}

/// The person chooses files in the kernel's trusted prompt, outside this call; each becomes a new
/// document. A file that cannot be imported is reported with the reason and the others go on.
pub async fn import(folder: &str) -> Result<Value, AppError> {
    let grants = host::pick_file(host::Pick {
        multiple: true,
        accept: IMPORTS
            .iter()
            .map(|(suffix, _)| (*suffix).to_owned())
            .collect(),
    })
    .await?;
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    for grant in &grants {
        match import_one(grant, folder).await {
            Ok(summary) => imported.push(summary),
            Err((name, failure)) => skipped.push(json!({"name": name, "reason": failure.message})),
        }
    }
    Ok(json!({"imported": imported, "skipped": skipped}))
}

async fn import_one(grant: &str, folder: &str) -> Result<Value, (Option<String>, AppError)> {
    let staged = format!("imports/{}", store::nonce());
    if let Some(parent) = store::on_disk(&staged).parent() {
        fs::create_dir_all(parent).map_err(|e| (None, error("FILE_FAILED", e.to_string())))?;
    }
    let outcome = async {
        let (_, name) = files::import(grant.to_owned(), staged.clone())
            .await
            .map_err(|failure| (None, failure))?;
        let content = fs::read_to_string(store::on_disk(&staged)).map_err(|_| {
            (
                Some(name.clone()),
                error("INVALID_ARGUMENT", format!("{name} is not UTF-8 text")),
            )
        })?;
        let (title, kind) = title_and_kind(&name);
        let new = NewDoc {
            id: store::nonce(),
            title,
            kind,
            folder: folder.to_owned(),
            content,
        };
        index::change(|docs| model::create(docs, new.clone(), &store::stamp()))
            .await
            .map_err(|failure| (Some(name), failure))
    }
    .await;
    let _ = fs::remove_file(store::on_disk(&staged));
    outcome
}

/// The file name the person is offered: the title, safe to save under.
pub fn export_name(title: &str, kind: Kind) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " ._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe: String = safe.trim().chars().take(120).collect();
    format!(
        "{}.{}",
        if safe.is_empty() { "document" } else { &safe },
        kind.extension()
    )
}

/// Stages a copy named after the document and hands it to the file broker, which keeps its own
/// copy of the offered bytes; the person saves it from the trusted prompt.
pub async fn export(id: &str) -> Result<Value, AppError> {
    let (docs, _) = index::load().await?;
    let doc = model::find(&docs, id)?;
    let content = store::read(doc, doc.revision)?;
    let name = export_name(&doc.title, doc.kind);
    // A folder of its own, so two exports of the same title never share a staged file.
    let staged = format!("exports/{}/{name}", store::nonce());
    let offered = async {
        store::replace(&staged, content.into_bytes()).await?;
        files::export(staged.clone()).await
    }
    .await;
    if let Some(folder) = store::on_disk(&staged).parent() {
        let _ = fs::remove_dir_all(folder);
    }
    offered?;
    Ok(json!({"offered": name}))
}

/// Every document write goes through the SDK (state and atomic replace), which a quiescent
/// snapshot holds, so the copy is consistent.
pub async fn snapshot(name: &str) -> Result<Value, AppError> {
    let info = files::snapshot(name.to_owned(), files::Consistency::Quiescent).await?;
    Ok(json!({
        "snapshotId": info.snapshot_id, "consistency": "quiescent",
        "files": info.files, "bytes": info.bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_imported_file_is_titled_after_its_name_and_typed_by_its_suffix() {
        assert_eq!(title_and_kind("Notes.MD"), ("Notes".into(), Kind::Markdown));
        assert_eq!(title_and_kind("page.htm"), ("page".into(), Kind::Html));
        assert_eq!(title_and_kind("a.b.txt"), ("a.b".into(), Kind::Markdown));
        assert_eq!(title_and_kind(".md"), (".md".into(), Kind::Markdown));
        assert_eq!(title_and_kind("plain"), ("plain".into(), Kind::Markdown));
    }

    #[test]
    fn an_offered_name_cannot_carry_a_path() {
        assert_eq!(export_name("a/b\\c:d", Kind::Markdown), "a_b_c_d.md");
        assert_eq!(export_name("  ", Kind::Html), "document.html");
        assert_eq!(export_name("Résumé 2026", Kind::Html), "Résumé 2026.html");
        assert_eq!(export_name(&"x".repeat(300), Kind::Markdown).len(), 123);
    }
}
