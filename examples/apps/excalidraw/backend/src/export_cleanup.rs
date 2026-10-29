use std::{io, path::Path};

pub fn remove(path: &Path) -> io::Result<()> {
    // This directory belongs to one export. A cancelled broker replacement may
    // still pin a temporary file inside it while the durable writer drains.
    // Unlink that entry too: publication requires the prepared inode to remain
    // at its original name, and removing the parent prevents later creation.
    std::fs::remove_dir_all(path.parent().expect("reserved export directory"))
}
