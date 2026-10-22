//! MP-08 / MP-10 / MP-11: public git repository import. The commit a fetch
//! resolves is the only revision a source-backed package may carry; the owner
//! reviews that commit and installs pin exactly it.
use super::{
    catalog::{failure, safe_locator},
    frontmatter::parse_frontmatter,
    package::{package_skill_directory, package_skill_directory_beneath},
    storage::SkillCatalogStorage,
    v2::{RepositorySkill, RepositorySkillSkip},
    CharioxSkillPackage,
};
use crate::error::DaemonError;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SKILL_DIRECTORIES: usize = 200;
const MAX_DEPTH: usize = 6;
const MAX_SKILL_MD_BYTES: u64 = 512 * 1024;

/// A private working tree (no `.git`) of one commit, removed on drop.
pub(crate) struct RepositoryCheckout {
    dir: PathBuf,
    pub revision: String,
    pub locator: String,
}

impl Drop for RepositoryCheckout {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// One checkout per `(url, revision)` for the duration of a preview, so a
/// repository installed as N members is fetched once.
#[derive(Default)]
pub(crate) struct RepositoryCheckouts(BTreeMap<(String, String), RepositoryCheckout>);

impl RepositoryCheckouts {
    pub fn get(
        &mut self,
        storage: &SkillCatalogStorage,
        url: &str,
        revision: &str,
    ) -> Result<&RepositoryCheckout, DaemonError> {
        let key = (url.to_string(), revision.to_string());
        if !self.0.contains_key(&key) {
            let checkout =
                RepositoryCheckout::fetch(staging_directory(storage)?, url, Some(revision))?;
            self.0.insert(key.clone(), checkout);
        }
        Ok(&self.0[&key])
    }
}

pub(crate) fn staging_directory(storage: &SkillCatalogStorage) -> Result<PathBuf, DaemonError> {
    storage.directory(
        &Path::new("state/skill-staging").join(format!("repo-{:032x}", rand::random::<u128>())),
    )
}

fn is_commit_id(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64)
        && revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validated_url(url: &str) -> Result<String, DaemonError> {
    let parsed = url::Url::parse(url).map_err(|_| failure("invalid repository URL"))?;
    match parsed.scheme() {
        "https" | "http" => {
            if parsed.host_str().is_none() {
                return Err(failure("repository URL needs a host"));
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(failure(
                    "only public repositories can be imported; remove the credentials from the URL",
                ));
            }
        }
        "file" => {}
        _ => return Err(failure("repository URL must use https")),
    }
    // `safe_locator` strips query and fragment; git gets the same URL.
    safe_locator(url)
}

fn run_git(directory: &Path, args: &[&str]) -> Result<String, DaemonError> {
    let mut child = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null", "-c", "credential.helper="])
        .args(args)
        .current_dir(directory)
        // Nothing the user configured may widen the import: no ssh rewrites, no
        // credential prompts, no inherited repository location.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "file:http:https")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                failure("git is required to import a repository; install git and retry")
            } else {
                failure("cannot start git to import the repository")
            }
        })?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < FETCH_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure(
                    "fetching the repository took longer than 2 minutes; try a smaller repository",
                ));
            }
            Err(_) => {
                return Err(failure(
                    "cannot wait for git while importing the repository",
                ))
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.by_ref().take(64 * 1024).read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.by_ref().take(64 * 1024).read_to_string(&mut stderr);
    }
    if status.success() {
        return Ok(stdout);
    }
    let lower = stderr.to_ascii_lowercase();
    if [
        "terminal prompts disabled",
        "could not read username",
        "authentication failed",
        "access denied",
        "repository not found",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Err(failure(
            "the repository needs authentication or does not exist; only public repositories can be imported",
        ));
    }
    let reason: String = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("git failed")
        .chars()
        .take(160)
        .collect();
    Err(failure(format!("cannot fetch the repository: {reason}")))
}

impl RepositoryCheckout {
    /// Fetches the repository's current commit, or exactly `revision` when the
    /// owner already reviewed one. `directory` is an empty private directory.
    pub fn fetch(
        directory: PathBuf,
        url: &str,
        revision: Option<&str>,
    ) -> Result<Self, DaemonError> {
        // Owned first, so every refusal below removes the staging directory.
        let mut checkout = RepositoryCheckout {
            dir: directory,
            revision: String::new(),
            locator: String::new(),
        };
        let url = validated_url(url)?;
        checkout.locator = safe_locator(&url)?;
        let dir = checkout.dir.clone();
        let fetched = match revision {
            Some(revision) => {
                if !is_commit_id(revision) {
                    return Err(failure("repository revision must be a full commit id"));
                }
                run_git(&dir, &["init", "--quiet", "--template="])?;
                run_git(
                    &dir,
                    &[
                        "fetch",
                        "--quiet",
                        "--depth",
                        "1",
                        "--no-tags",
                        "--",
                        &url,
                        revision,
                    ],
                )?;
                run_git(&dir, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
                revision.to_string()
            }
            None => {
                run_git(
                    &dir,
                    &[
                        "clone",
                        "--quiet",
                        "--depth",
                        "1",
                        "--no-tags",
                        "--",
                        &url,
                        ".",
                    ],
                )?;
                String::new()
            }
        };
        let head = run_git(&dir, &["rev-parse", "HEAD"])?.trim().to_string();
        if !is_commit_id(&head) || (!fetched.is_empty() && head != fetched) {
            return Err(failure(
                "the repository did not return the reviewed commit; resolve it again",
            ));
        }
        fs::remove_dir_all(dir.join(".git"))
            .map_err(|_| failure("cannot finish the repository checkout"))?;
        checkout.revision = head;
        Ok(checkout)
    }

    /// Every directory holding a SKILL.md, in path order. A skill directory owns
    /// everything below it, so nested directories are not listed again.
    pub fn skills(&self) -> Result<(Vec<RepositorySkill>, Vec<RepositorySkillSkip>), DaemonError> {
        let mut found = vec![];
        collect_skill_directories(&self.dir, "", 0, &mut found)?;
        let mut skills = vec![];
        let mut skipped = vec![];
        for path in found {
            let file = self.dir.join(&path).join("SKILL.md");
            let parsed = fs::metadata(&file)
                .map_err(|_| failure("SKILL.md is unreadable"))
                .and_then(|meta| {
                    if meta.len() > MAX_SKILL_MD_BYTES {
                        Err(failure("SKILL.md must be at most 512 KiB"))
                    } else {
                        fs::read_to_string(&file).map_err(|_| failure("SKILL.md must be UTF-8"))
                    }
                })
                .and_then(|body| parse_frontmatter(&body));
            match parsed {
                Ok(front) => skills.push(RepositorySkill {
                    path,
                    name: front.name,
                    description: front.description,
                }),
                Err(error) => skipped.push(RepositorySkillSkip {
                    path,
                    reason: match error {
                        DaemonError::LocalTransport { message, .. } => message,
                        other => other.to_string(),
                    },
                }),
            }
        }
        Ok((skills, skipped))
    }

    /// The immutable package of one member directory, read without following
    /// links out of the checkout.
    pub fn package(&self, path: &str) -> Result<CharioxSkillPackage, DaemonError> {
        if path.is_empty() {
            return package_skill_directory(&self.dir);
        }
        if path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
            || path.contains('\\')
            || path.contains('\0')
        {
            return Err(failure("invalid repository skill path"));
        }
        package_skill_directory_beneath(&self.dir, &self.dir.join(path))
    }
}

fn collect_skill_directories(
    root: &Path,
    relative: &str,
    depth: usize,
    found: &mut Vec<String>,
) -> Result<(), DaemonError> {
    let directory = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let marker = fs::symlink_metadata(directory.join("SKILL.md"));
    if marker.map(|meta| meta.is_file()).unwrap_or(false) {
        found.push(relative.to_string());
        if found.len() > MAX_SKILL_DIRECTORIES {
            return Err(failure(format!(
                "the repository has more than {MAX_SKILL_DIRECTORIES} skills; import a smaller repository"
            )));
        }
        return Ok(());
    }
    if depth >= MAX_DEPTH {
        return Ok(());
    }
    let mut children: Vec<String> = fs::read_dir(&directory)
        .map_err(|_| failure("cannot read the repository checkout"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != ".git" && name != "node_modules")
        .collect();
    children.sort();
    for child in children {
        let next = if relative.is_empty() {
            child
        } else {
            format!("{relative}/{child}")
        };
        collect_skill_directories(root, &next, depth + 1, found)?;
    }
    Ok(())
}
