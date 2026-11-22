//! Which Vault entry each Project Secret was filled from. Ids only, never a value: the copy lives in the
//! Project's Vault service, and a change of the entry re-fills it through this link.
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

/// Project id -> Secret name -> Vault entry id.
type Links = BTreeMap<String, BTreeMap<String, String>>;

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

    fn change(&self, edit: impl FnOnce(&mut Links)) -> Result<(), DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut links = self.read()?;
        edit(&mut links);
        links.retain(|_, names| !names.is_empty());
        self.write(&links)
    }

    /// Secret name -> Vault entry id, for one Project.
    pub(crate) fn of_project(
        &self,
        project: &str,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        Ok(self.read()?.remove(project).unwrap_or_default())
    }

    pub(crate) fn set(&self, project: &str, name: &str, entry_id: &str) -> Result<(), DaemonError> {
        self.change(|links| {
            links
                .entry(project.to_owned())
                .or_default()
                .insert(name.to_owned(), entry_id.to_owned());
        })
    }

    pub(crate) fn remove(&self, project: &str, name: &str) -> Result<(), DaemonError> {
        self.change(|links| {
            if let Some(names) = links.get_mut(project) {
                names.remove(name);
            }
        })
    }

    /// (Project id, Secret name) of every Secret filled from this entry.
    pub(crate) fn filled_from(&self, entry_id: &str) -> Result<Vec<(String, String)>, DaemonError> {
        Ok(self
            .read()?
            .into_iter()
            .flat_map(|(project, names)| {
                names
                    .into_iter()
                    .filter(|(_, id)| id == entry_id)
                    .map(move |(name, _)| (project.clone(), name))
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
        assert!(links.of_project("p").unwrap().is_empty());
        links.set("p", "GH_TOKEN", "cap-1").unwrap();
        links.set("q", "OTHER", "cap-1").unwrap();
        links.set("p", "KEY", "cap-2").unwrap();
        assert_eq!(links.of_project("p").unwrap().len(), 2);
        let mut found = links.filled_from("cap-1").unwrap();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("p".into(), "GH_TOKEN".into()),
                ("q".into(), "OTHER".into())
            ]
        );
        links.remove("p", "GH_TOKEN").unwrap();
        assert_eq!(links.filled_from("cap-1").unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
