//! What stands behind each Project Secret: the Vault entry of this kernel it was filled from, or the
//! kernel it was copied from that holds it linked there. Ids only, never a value: the copy lives in the
//! Project's Vault service, and a change of the entry (here, or on the source at the next Update)
//! re-fills it through this link. Every write of a linked Secret's value runs under the same lock as
//! the link change, so a typed value always wins over a re-fill.
use super::environment_error;
use crate::error::DaemonError;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE: &str = "project-environment-links.json";
const MAX_BYTES: u64 = 1024 * 1024;

/// One kernel process owns its home, so one lock serializes every read-modify-write.
static WRITES: Mutex<()> = Mutex::new(());

/// The one origin of a Secret's value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecretLink {
    /// Filled from this kernel's Vault entry of that id.
    Entry(String),
    /// Copied from the kernel of that id, where the Secret is linked.
    Peer(String),
}

/// Project id -> Secret name -> link.
type Links = BTreeMap<String, BTreeMap<String, SecretLink>>;

#[derive(Debug, Clone)]
pub(crate) struct SecretLinks {
    path: PathBuf,
}

impl SecretLinks {
    pub(crate) fn new(private_state_root: &Path) -> Self {
        Self {
            path: private_state_root.join(FILE),
        }
    }

    fn read(&self) -> Result<Links, DaemonError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Links::new()),
            Err(_) => return Err(environment_error("the Secret links are unreadable")),
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("the Secret links are unreadable"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(environment_error("the Secret links exceed their bound"));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("the Secret links are not valid"))
    }

    fn write(&self, links: &Links) -> Result<(), DaemonError> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| environment_error("the Secret links directory is unavailable"))?;
        fs::create_dir_all(directory)
            .map_err(|_| environment_error("the Secret links directory is unavailable"))?;
        let bytes = serde_json::to_vec(links)
            .map_err(|_| environment_error("the Secret links could not be encoded"))?;
        let temporary = directory.join(format!(".tmp-links-{}", rand::random::<u64>()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            File::open(directory)?.sync_all()
        })();
        let _ = fs::remove_file(&temporary);
        result.map_err(|_| environment_error("saving the Secret links failed"))
    }

    fn change<T>(
        &self,
        edit: impl FnOnce(&mut Links) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut links = self.read()?;
        let before = links.clone();
        let outcome = edit(&mut links)?;
        links.retain(|_, names| !names.is_empty());
        if links != before {
            self.write(&links)?;
        }
        Ok(outcome)
    }

    pub(crate) fn get(&self, project: &str, name: &str) -> Result<Option<SecretLink>, DaemonError> {
        Ok(self
            .read()?
            .get(project)
            .and_then(|names| names.get(name))
            .cloned())
    }

    /// Runs `write`; when it stored the value (returns true), `link` stands behind the Secret, and
    /// with no link any earlier one ends.
    pub(crate) fn write_linked(
        &self,
        project: &str,
        name: &str,
        link: Option<SecretLink>,
        write: impl FnOnce() -> Result<bool, DaemonError>,
    ) -> Result<bool, DaemonError> {
        self.change(|links| {
            if !write()? {
                return Ok(false);
            }
            let names = links.entry(project.to_owned()).or_default();
            match link {
                Some(link) => names.insert(name.to_owned(), link),
                None => names.remove(name),
            };
            Ok(true)
        })
    }

    /// Runs `write` only while `expected` still stands behind the Secret; false when it no longer does.
    pub(crate) fn write_if_linked(
        &self,
        project: &str,
        name: &str,
        expected: &SecretLink,
        write: impl FnOnce() -> Result<(), DaemonError>,
    ) -> Result<bool, DaemonError> {
        self.change(|links| {
            if links.get(project).and_then(|names| names.get(name)) != Some(expected) {
                return Ok(false);
            }
            write()?;
            Ok(true)
        })
    }

    /// Ends the link when it is still `expected`; the value stays.
    pub(crate) fn unlink_if(
        &self,
        project: &str,
        name: &str,
        expected: &SecretLink,
    ) -> Result<(), DaemonError> {
        self.change(|links| {
            if let Some(names) = links.get_mut(project) {
                if names.get(name) == Some(expected) {
                    names.remove(name);
                }
            }
            Ok(())
        })
    }

    /// The entry is gone: every Secret filled from it keeps its value as a normal Secret.
    pub(crate) fn unlink_entry(&self, entry_id: &str) -> Result<(), DaemonError> {
        let gone = SecretLink::Entry(entry_id.to_owned());
        self.change(|links| {
            for names in links.values_mut() {
                names.retain(|_, link| *link != gone);
            }
            Ok(())
        })
    }

    /// The Secrets of a Project that have something standing behind them, for a kernel copying them.
    pub(crate) fn linked_names(&self, project: &str) -> Result<Vec<String>, DaemonError> {
        Ok(self
            .read()?
            .remove(project)
            .map(|names| names.into_keys().collect())
            .unwrap_or_default())
    }

    /// (Project id, Secret name) of every Secret filled from this entry.
    pub(crate) fn filled_from(&self, entry_id: &str) -> Result<Vec<(String, String)>, DaemonError> {
        let entry = SecretLink::Entry(entry_id.to_owned());
        Ok(self
            .read()?
            .into_iter()
            .flat_map(|(project, names)| {
                names
                    .into_iter()
                    .filter(|(_, link)| *link == entry)
                    .map(move |(name, _)| (project.clone(), name))
                    .collect::<Vec<_>>()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_kept_per_project_and_found_by_their_entry() {
        let dir =
            std::env::temp_dir().join(format!("chariox-links-{:032x}", rand::random::<u128>()));
        let links = SecretLinks::new(&dir);
        let set = |project: &str, name: &str, link: SecretLink| {
            assert!(links
                .write_linked(project, name, Some(link), || Ok(true))
                .unwrap());
        };
        set("p", "GH_TOKEN", SecretLink::Entry("cap-1".into()));
        set("q", "OTHER", SecretLink::Entry("cap-1".into()));
        set("p", "KEY", SecretLink::Entry("cap-2".into()));
        set("p", "COPIED", SecretLink::Peer("cap-1".into()));
        let mut found = links.filled_from("cap-1").unwrap();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("p".into(), "GH_TOKEN".into()),
                ("q".into(), "OTHER".into())
            ]
        );
        assert_eq!(
            links.linked_names("p").unwrap(),
            vec!["COPIED".to_owned(), "GH_TOKEN".into(), "KEY".into()]
        );
        // A typed value ends the link; a write that stored nothing changes nothing.
        assert!(!links
            .write_linked("p", "KEY", None, || Ok(false))
            .unwrap());
        assert!(links.write_linked("p", "GH_TOKEN", None, || Ok(true)).unwrap());
        assert_eq!(links.filled_from("cap-1").unwrap().len(), 1);
        // A re-fill only writes while its link still stands.
        let peer = SecretLink::Peer("cap-1".into());
        assert!(links.write_if_linked("p", "COPIED", &peer, || Ok(())).unwrap());
        assert!(!links
            .write_if_linked("p", "GH_TOKEN", &peer, || panic!("not linked"))
            .unwrap());
        links.unlink_entry("cap-2").unwrap();
        links.unlink_if("p", "COPIED", &peer).unwrap();
        assert!(links.linked_names("p").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
