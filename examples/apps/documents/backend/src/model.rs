//! The document index: the one state value that names every document and its kept revisions, and
//! the pure rules that change it. A rule returns the next index plus the content file to write
//! first and the files to prune after the index commits, so a committed entry always names an
//! existing file and a lost compare-and-set leaves nothing the winner relies on.

use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

/// The kernel bounds the index value (256 KiB) and its JSON node count (16384). An entry is about
/// 0.6 KiB and about 38 nodes with full history, so 300 documents stay well inside both.
pub const MAX_INDEX_BYTES: usize = 256 * 1024;
pub const MAX_DOCUMENTS: usize = 300;
pub const MAX_CONTENT: usize = 512 * 1024;
pub const KEEP_VERSIONS: usize = 10;
const MAX_NAME_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Markdown,
    Html,
}

impl Kind {
    pub fn parse(name: &str) -> Result<Kind, AppError> {
        match name {
            "markdown" => Ok(Kind::Markdown),
            "html" => Ok(Kind::Html),
            _ => Err(error("INVALID_ARGUMENT", "kind is markdown or html")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Markdown => "markdown",
            Kind::Html => "html",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Kind::Markdown => "md",
            Kind::Html => "html",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub revision: u64,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub kind: Kind,
    pub revision: u64,
    pub versions: Vec<Version>,
    pub updated_at_ms: u64,
}

impl Doc {
    fn from_value(value: &Value) -> Result<Doc, AppError> {
        let invalid = || error("INVALID_STATE", "the document index is damaged");
        let text = |key: &str| value[key].as_str().map(str::to_owned).ok_or_else(invalid);
        let number = |value: &Value, key: &str| value[key].as_u64().ok_or_else(invalid);
        Ok(Doc {
            id: text("id")?,
            title: text("title")?,
            folder: text("folder")?,
            kind: Kind::parse(&text("kind")?).map_err(|_| invalid())?,
            revision: number(value, "revision")?,
            versions: value["versions"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|version| {
                    Ok(Version {
                        revision: number(version, "revision")?,
                        file: version["file"].as_str().ok_or_else(invalid)?.to_owned(),
                    })
                })
                .collect::<Result<_, AppError>>()?,
            updated_at_ms: number(value, "updated_at_ms")?,
        })
    }

    fn to_value(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "folder": self.folder, "kind": self.kind.name(),
            "revision": self.revision, "updated_at_ms": self.updated_at_ms,
            "versions": self.versions.iter()
                .map(|v| json!({"revision": v.revision, "file": v.file})).collect::<Vec<_>>(),
        })
    }

    /// What the tools and the view show of a document.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "folder": self.folder, "kind": self.kind.name(),
            "revision": self.revision, "updated_at_ms": self.updated_at_ms,
            "versions": self.versions.iter().map(|v| v.revision).collect::<Vec<_>>(),
        })
    }

    pub fn file_of(&self, revision: u64) -> Option<&str> {
        self.versions
            .iter()
            .find(|version| version.revision == revision)
            .map(|version| version.file.as_str())
    }

    pub fn content_path(&self, file: &str) -> String {
        content_path(&self.id, file)
    }
}

pub fn content_path(id: &str, file: &str) -> String {
    format!("documents/{id}/{file}")
}

pub fn parse_index(text: &str) -> Result<Vec<Doc>, AppError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|_| error("INVALID_STATE", "the document index is damaged"))?;
    value["docs"]
        .as_array()
        .ok_or_else(|| error("INVALID_STATE", "the document index is damaged"))?
        .iter()
        .map(Doc::from_value)
        .collect()
}

pub fn index_text(docs: &[Doc]) -> String {
    json!({"docs": docs.iter().map(Doc::to_value).collect::<Vec<_>>()}).to_string()
}

/// A content file the App writes before the index names it.
#[derive(Debug, PartialEq)]
pub struct FileWrite {
    pub path: String,
    pub content: String,
}

/// What to remove once the index has committed.
#[derive(Debug, PartialEq)]
pub enum Prune {
    /// Revisions below the oldest kept one. A concurrent save's uncommitted file is a newer
    /// revision, so it is never touched.
    Older {
        id: String,
        keep: Vec<String>,
    },
    Folder(String),
}

#[derive(Debug)]
pub struct Plan {
    pub docs: Vec<Doc>,
    pub write: Option<FileWrite>,
    pub prune: Prune,
    pub summary: Value,
    /// The change the view's feed records with the commit.
    pub delta: Value,
}

/// Fresh values a rule cannot make itself: the clock and a random suffix per written file.
pub struct Stamp {
    pub now_ms: u64,
    pub nonce: String,
}

#[derive(Clone)]
pub struct NewDoc {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub folder: String,
    pub content: String,
}

pub struct Revision {
    pub content: Option<String>,
    pub title: Option<String>,
    pub folder: Option<String>,
}

fn limit(message: &str) -> AppError {
    error("LIMIT_EXCEEDED", message)
}

fn check_name(what: &str, value: &str, allow_empty: bool) -> Result<(), AppError> {
    if value.chars().count() > MAX_NAME_CHARS || (!allow_empty && value.trim().is_empty()) {
        return Err(error(
            "INVALID_ARGUMENT",
            format!("{what} is at most {MAX_NAME_CHARS} characters and a title is not empty"),
        ));
    }
    Ok(())
}

fn check_content(content: &str) -> Result<(), AppError> {
    if content.len() > MAX_CONTENT {
        return Err(limit("Document is larger than 512 KiB"));
    }
    Ok(())
}

fn fits(docs: &[Doc]) -> Result<(), AppError> {
    if index_text(docs).len() > MAX_INDEX_BYTES {
        return Err(limit(
            "The document index is full; delete documents or shorten titles and folders",
        ));
    }
    Ok(())
}

pub fn find<'a>(docs: &'a [Doc], id: &str) -> Result<&'a Doc, AppError> {
    docs.iter()
        .find(|doc| doc.id == id)
        .ok_or_else(|| error("NOT_FOUND", format!("No document with id {id}")))
}

fn kept(doc: &Doc) -> Vec<String> {
    doc.versions.iter().map(|v| v.file.clone()).collect()
}

fn changed(doc: &Doc) -> Value {
    json!({"id": doc.id, "revision": doc.revision})
}

pub fn create(docs: &[Doc], new: NewDoc, stamp: &Stamp) -> Result<Plan, AppError> {
    check_name("title", &new.title, false)?;
    check_name("folder", &new.folder, true)?;
    check_content(&new.content)?;
    if docs.len() >= MAX_DOCUMENTS {
        return Err(limit(&format!("At most {MAX_DOCUMENTS} documents")));
    }
    let file = format!("1-{}.{}", stamp.nonce, new.kind.extension());
    let doc = Doc {
        id: new.id,
        title: new.title,
        folder: new.folder,
        kind: new.kind,
        revision: 1,
        versions: vec![Version {
            revision: 1,
            file: file.clone(),
        }],
        updated_at_ms: stamp.now_ms,
    };
    let mut next = docs.to_vec();
    next.push(doc.clone());
    fits(&next)?;
    Ok(Plan {
        docs: next,
        write: Some(FileWrite {
            path: doc.content_path(&file),
            content: new.content,
        }),
        prune: Prune::Older {
            id: doc.id.clone(),
            keep: kept(&doc),
        },
        summary: doc.summary(),
        delta: changed(&doc),
    })
}

/// Every change, a rename or move included, is a new revision, so a stale editor can never
/// silently revert another person's or agent's edit. `expected` is the revision the caller
/// started from; a mismatch is a conflict it resolves by reading again, never a merge.
pub fn revise(
    docs: &[Doc],
    id: &str,
    expected: u64,
    change: Revision,
    stamp: &Stamp,
) -> Result<Plan, AppError> {
    let mut next = docs.to_vec();
    let doc = next
        .iter_mut()
        .find(|doc| doc.id == id)
        .ok_or_else(|| error("NOT_FOUND", format!("No document with id {id}")))?;
    if doc.revision != expected {
        return Err(error(
            "CONFLICT",
            format!(
                "Document changed (now revision {}); reload before saving",
                doc.revision
            ),
        ));
    }
    if let Some(title) = &change.title {
        check_name("title", title, false)?;
    }
    if let Some(folder) = &change.folder {
        check_name("folder", folder, true)?;
    }
    if let Some(content) = &change.content {
        check_content(content)?;
    }
    let previous = doc
        .file_of(doc.revision)
        .ok_or_else(|| error("INVALID_STATE", "the document index is damaged"))?
        .to_owned();
    if let Some(title) = change.title {
        doc.title = title;
    }
    if let Some(folder) = change.folder {
        doc.folder = folder;
    }
    doc.revision += 1;
    let (file, write) = match change.content {
        Some(content) => {
            let file = format!("{}-{}.{}", doc.revision, stamp.nonce, doc.kind.extension());
            let path = doc.content_path(&file);
            (file, Some(FileWrite { path, content }))
        }
        None => (previous, None),
    };
    doc.versions.push(Version {
        revision: doc.revision,
        file,
    });
    let excess = doc.versions.len().saturating_sub(KEEP_VERSIONS);
    doc.versions.drain(..excess);
    doc.updated_at_ms = stamp.now_ms;
    let doc = doc.clone();
    fits(&next)?;
    Ok(Plan {
        docs: next,
        write,
        prune: Prune::Older {
            id: doc.id.clone(),
            keep: kept(&doc),
        },
        summary: doc.summary(),
        delta: changed(&doc),
    })
}

pub fn remove(docs: &[Doc], id: &str) -> Result<Plan, AppError> {
    find(docs, id)?;
    Ok(Plan {
        docs: docs.iter().filter(|doc| doc.id != id).cloned().collect(),
        write: None,
        prune: Prune::Folder(id.to_owned()),
        summary: json!({"deleted": id}),
        delta: json!({"id": id, "deleted": true}),
    })
}

/// The file names below the oldest kept revision that `keep` no longer names.
pub fn stale<'a>(names: &'a [String], keep: &[String]) -> Vec<&'a str> {
    let revision = |name: &str| name.split('-').next().and_then(|n| n.parse::<u64>().ok());
    let Some(oldest) = keep.iter().filter_map(|name| revision(name)).min() else {
        return Vec::new();
    };
    names
        .iter()
        .filter(|name| !keep.contains(name) && revision(name).is_some_and(|r| r < oldest))
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(n: u64) -> Stamp {
        Stamp {
            now_ms: 1000 + n,
            nonce: format!("n{n}"),
        }
    }

    fn doc_with(content: &str) -> Vec<Doc> {
        let new = NewDoc {
            id: "d1".into(),
            title: "Plan".into(),
            kind: Kind::Markdown,
            folder: "work".into(),
            content: content.into(),
        };
        create(&[], new, &stamp(0)).unwrap().docs
    }

    fn edit(docs: &[Doc], expected: u64, content: &str, n: u64) -> Result<Plan, AppError> {
        let change = Revision {
            content: Some(content.into()),
            title: None,
            folder: None,
        };
        revise(docs, "d1", expected, change, &stamp(n))
    }

    #[test]
    fn a_created_document_is_revision_one_with_its_file_named_by_revision_and_suffix() {
        let new = NewDoc {
            id: "d1".into(),
            title: "Plan".into(),
            kind: Kind::Html,
            folder: String::new(),
            content: "<p>x</p>".into(),
        };
        let plan = create(&[], new, &stamp(0)).unwrap();
        assert_eq!(plan.write.unwrap().path, "documents/d1/1-n0.html");
        assert_eq!(plan.summary["revision"], 1);
        assert_eq!(plan.delta, json!({"id": "d1", "revision": 1}));
    }

    #[test]
    fn the_index_keeps_the_state_shape_the_old_runtime_stored() {
        let docs = doc_with("a");
        let text = index_text(&docs);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["docs"][0],
            json!({"id": "d1", "title": "Plan", "folder": "work", "kind": "markdown", "revision": 1,
                "updated_at_ms": 1000, "versions": [{"revision": 1, "file": "1-n0.md"}]})
        );
        assert_eq!(parse_index(&text).unwrap(), docs);
    }

    #[test]
    fn a_stale_editor_gets_a_conflict_not_a_merge() {
        let docs = edit(&doc_with("a"), 1, "b", 1).unwrap().docs;
        let refused = edit(&docs, 1, "c", 2).unwrap_err();
        assert_eq!(refused.code, "CONFLICT");
        assert!(
            refused.message.contains("now revision 2"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn history_keeps_ten_revisions_and_prunes_only_older_files() {
        let mut docs = doc_with("a");
        let mut last = None;
        for n in 1..=12u64 {
            let plan = edit(&docs, n, "x", n).unwrap();
            docs = plan.docs.clone();
            last = Some(plan);
        }
        let plan = last.unwrap();
        assert_eq!(docs[0].versions.len(), KEEP_VERSIONS);
        assert_eq!(docs[0].versions[0].revision, 4);
        let Prune::Older { keep, .. } = plan.prune else {
            panic!("an edit prunes older files")
        };
        let on_disk: Vec<String> = (1..=14).map(|r| format!("{r}-n{}.md", r - 1)).collect();
        let on_disk: Vec<String> = on_disk
            .into_iter()
            .chain(["99-newer.md".to_owned()])
            .collect();
        let removed = stale(&on_disk, &keep);
        assert!(
            removed.contains(&"1-n0.md") && removed.contains(&"3-n2.md"),
            "{removed:?}"
        );
        assert!(
            !removed.contains(&"99-newer.md"),
            "a concurrent newer save is never removed"
        );
        assert!(!removed.iter().any(|name| keep.iter().any(|k| k == name)));
    }

    #[test]
    fn a_rename_is_a_new_revision_that_reuses_the_content_file() {
        let docs = doc_with("a");
        let change = Revision {
            content: None,
            title: Some("Renamed".into()),
            folder: None,
        };
        let plan = revise(&docs, "d1", 1, change, &stamp(1)).unwrap();
        assert!(plan.write.is_none());
        assert_eq!(plan.docs[0].revision, 2);
        assert_eq!(plan.docs[0].file_of(2), plan.docs[0].file_of(1));
        assert_eq!(plan.docs[0].title, "Renamed");
    }

    #[test]
    fn size_and_count_limits_are_refused_before_anything_is_written() {
        let big = "x".repeat(MAX_CONTENT + 1);
        assert_eq!(
            edit(&doc_with("a"), 1, &big, 1).unwrap_err().code,
            "LIMIT_EXCEEDED"
        );
        let mut docs = Vec::new();
        for n in 0..MAX_DOCUMENTS {
            let new = NewDoc {
                id: format!("d{n}"),
                title: "t".into(),
                kind: Kind::Markdown,
                folder: String::new(),
                content: String::new(),
            };
            docs = create(&docs, new, &stamp(0)).unwrap().docs;
        }
        let new = NewDoc {
            id: "more".into(),
            title: "t".into(),
            kind: Kind::Markdown,
            folder: String::new(),
            content: String::new(),
        };
        assert_eq!(
            create(&docs, new, &stamp(0)).unwrap_err().code,
            "LIMIT_EXCEEDED"
        );
    }

    #[test]
    fn an_index_over_the_state_limit_is_refused() {
        let full = |n: usize| Doc {
            id: format!("{n:08x}"),
            title: "t".repeat(200),
            folder: "f".repeat(200),
            kind: Kind::Markdown,
            revision: 20,
            versions: (11..=20)
                .map(|revision| Version {
                    revision,
                    file: format!("{revision}-{n:08x}.md"),
                })
                .collect(),
            updated_at_ms: 1_700_000_000_000,
        };
        let docs: Vec<Doc> = (0..MAX_DOCUMENTS).map(full).collect();
        assert_eq!(fits(&docs).unwrap_err().code, "LIMIT_EXCEEDED");
        assert!(fits(&docs[..100]).is_ok());
    }

    #[test]
    fn deleting_names_the_whole_folder_for_removal() {
        let plan = remove(&doc_with("a"), "d1").unwrap();
        assert!(plan.docs.is_empty());
        assert_eq!(plan.prune, Prune::Folder("d1".into()));
        assert_eq!(remove(&plan.docs, "d1").unwrap_err().code, "NOT_FOUND");
    }
}
