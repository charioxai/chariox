//! Which file a workspace-relative path names. A copy stays inside the workspace through real
//! folders only, and never writes where Git or Chariox would run or trust what it finds.
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    /// Missing folders on the way are created.
    Write,
}

/// The file `relative` names inside `root`, or why a copy may not use it.
pub(crate) fn resolve(root: &Path, relative: &str, access: Access) -> Result<PathBuf, String> {
    let parts = parts(relative)?;
    if access == Access::Write {
        refuse_protected(relative, &parts)?;
    }
    let root = root
        .canonicalize()
        .map_err(|_| "The agent's workspace is not available on its kernel".to_owned())?;
    let (name, folders) = parts.split_last().expect("at least one part");
    let mut current = root.clone();
    for folder in folders {
        current.push(folder);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "`{relative}` passes through a link; name the file by its real path"
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!("`{relative}` passes through a file, not a folder"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if access == Access::Read {
                    return Err(format!("`{relative}` does not exist"));
                }
                std::fs::create_dir(&current)
                    .map_err(|_| format!("A folder of `{relative}` could not be created"))?;
            }
            Err(_) => return Err(format!("`{relative}` could not be reached")),
        }
    }
    // A folder swapped for a link while the walk ran must still not lead out.
    if !current
        .canonicalize()
        .is_ok_and(|parent| parent.starts_with(&root))
    {
        return Err(format!("`{relative}` leads out of the agent's workspace"));
    }
    let path = current.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "`{relative}` is a link; name the file it points to by its real path"
        )),
        Ok(metadata) if metadata.is_dir() => Err(format!(
            "`{relative}` is a folder; copy one file (pack a folder with tar first)"
        )),
        Ok(metadata) if access == Access::Read && !metadata.is_file() => {
            Err(format!("`{relative}` is not a regular file"))
        }
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && access == Access::Write => {
            Ok(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("`{relative}` does not exist"))
        }
        Err(_) => Err(format!("`{relative}` could not be reached")),
    }
}

/// The plain names along `relative`, which must stay below the workspace root.
fn parts(relative: &str) -> Result<Vec<&str>, String> {
    if Path::new(relative).is_absolute() {
        return Err(format!(
            "`{relative}` is absolute; name a path relative to the agent's workspace"
        ));
    }
    let mut parts = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| format!("`{relative}` is not valid text"))?,
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("`{relative}` leads out of the agent's workspace"));
            }
        }
    }
    if parts.is_empty() {
        return Err("Name a file inside the agent's workspace".to_owned());
    }
    Ok(parts)
}

/// Git runs hooks and reads config from `.git`, and Chariox trusts what it keeps in `.chariox*`,
/// so a copy never writes there. Case-insensitive volumes treat `.GIT` as `.git`.
fn refuse_protected(relative: &str, parts: &[&str]) -> Result<(), String> {
    if parts.iter().any(|part| part.eq_ignore_ascii_case(".git")) {
        return Err(format!(
            "`{relative}` is inside .git, where Git runs hooks and reads config; copy into the working tree and let Git fetch or apply it"
        ));
    }
    if parts
        .iter()
        .any(|part| part.to_ascii_lowercase().starts_with(".chariox"))
    {
        return Err(format!(
            "`{relative}` is one of Chariox's own files; choose another destination"
        ));
    }
    Ok(())
}
