//! The checkpoint stays outside the two guest preopens until the writer's
//! committed data generation advances. Every retry restores it before entry.
use super::*;
use crate::worker_process::private_data::PrivateData;

mod tree;
pub(super) use tree::clear;

fn failed() -> WorkerError {
    WorkerError::Storage("app_wasm_migration_snapshot")
}

fn discard(root: &Dir, name: &str) -> Result<(), WorkerError> {
    let name = OsStr::new(name);
    let child = match root.child(name) {
        Ok(dir) => dir,
        Err(crate::private_fs::FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(())
        }
        Err(_) => return Err(failed()),
    };
    tree::clear(&child)?;
    root.remove_directory(name).map_err(|_| failed())?;
    root.sync().map_err(|_| failed())
}

fn retire(root: &Dir) -> Result<(), WorkerError> {
    crate::private_fs::publish(root, OsStr::new(".migration"), OsStr::new(".retired"))
        .map_err(|_| failed())?;
    root.sync().map_err(|_| failed())?;
    discard(root, ".retired")
}

pub(super) fn prepare(
    root: &Dir,
    data: &PrivateData,
    committed: Option<u64>,
    migrating: bool,
) -> Result<(), WorkerError> {
    discard(root, ".retired")?;
    discard(root, ".pending")?;
    let snapshot = match root.child(OsStr::new(".migration")) {
        Ok(dir) => Some(dir),
        Err(crate::private_fs::FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(failed()),
    };
    if let Some(snapshot) = snapshot {
        let mut bytes = String::new();
        let generation = snapshot
            .read_file(OsStr::new("generation"), true)
            .map_err(|_| failed())?;
        if generation.metadata().map_err(|_| failed())?.len() > 20 {
            return Err(failed());
        }
        generation
            .take(21)
            .read_to_string(&mut bytes)
            .map_err(|_| failed())?;
        let origin: u64 = bytes.parse().map_err(|_| failed())?;
        if origin == 0 || origin.to_string() != bytes {
            return Err(failed());
        }
        let committed = committed.ok_or_else(failed)?;
        if origin == committed {
            let source = snapshot.child(OsStr::new("data")).map_err(|_| failed())?;
            let expected = tree::copy(&source, None)?;
            tree::clear(data.directory())?;
            if tree::copy(&source, Some(data.directory()))? != expected
                || tree::copy(data.directory(), None)? != expected
            {
                return Err(failed());
            }
            if data.generation() == committed {
                retire(root)?;
            }
            return Ok(());
        }
        if origin > committed {
            return Err(failed());
        }
        retire(root)?;
    }
    if !migrating {
        return Ok(());
    }
    if let Some(committed) = committed.filter(|generation| *generation != data.generation()) {
        if committed > data.generation() {
            return Err(failed());
        }
        let pending = root
            .create_child(OsStr::new(".pending"))
            .map_err(|_| failed())?;
        let target = pending
            .create_child(OsStr::new("data"))
            .map_err(|_| failed())?;
        tree::copy(data.directory(), Some(&target))?;
        pending
            .write_new(OsStr::new("generation"), committed.to_string().as_bytes())
            .map_err(|_| failed())?;
        pending.sync().map_err(|_| failed())?;
        crate::private_fs::publish(root, OsStr::new(".pending"), OsStr::new(".migration"))
            .map_err(|_| failed())?;
        root.sync().map_err(|_| failed())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
