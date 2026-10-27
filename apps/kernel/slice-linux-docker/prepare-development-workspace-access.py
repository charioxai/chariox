#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: share ordinary publication copies, retaining their owner."""
import os
from pathlib import Path
import pwd
import stat
import sys


def directory_metadata(path):
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError("publication path contains a non-directory or symlink")
    return metadata


def walk_error(error):
    raise error


def prepare(paths):
    worker = pwd.getpwnam("slice")
    roots = []
    # Validate all mounts before changing access. Never follow a symlink outside
    # the kernel-owned publication, including a symlink in its ancestor path.
    for value in paths:
        root = Path(value)
        if not root.is_absolute() or str(root) != value or root == Path("/"):
            raise ValueError("invalid publication root")
        for parent in reversed(root.parents):
            directory_metadata(parent)
        directory_metadata(root)
        roots.append(root)

    for root in roots:
        for parent in reversed(root.parents):
            metadata = directory_metadata(parent)
            mode = stat.S_IMODE(metadata.st_mode)
            search = (mode & stat.S_IXOTH or
                      metadata.st_uid == worker.pw_uid and mode & stat.S_IXUSR or
                      metadata.st_gid == worker.pw_gid and mode & stat.S_IXGRP)
            if not search:
                # These are container ancestors, not the host state hierarchy.
                # Give only the worker group search, without directory listing.
                os.chown(parent, -1, worker.pw_gid, follow_symlinks=False)
                os.chmod(parent, mode | stat.S_IXGRP)

        for directory, _, files in os.walk(root, followlinks=False, onerror=walk_error):
            for path in [Path(directory), *(Path(directory) / name for name in files)]:
                metadata = path.lstat()
                if stat.S_ISLNK(metadata.st_mode):
                    continue
                if not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
                    raise ValueError("publication contains an unsupported file")
                mode = stat.S_IMODE(metadata.st_mode)
                # A fresh publication is a copy. Share its owner's permissions
                # with the worker group; keep the source kernel's ownership and
                # existing world permissions, including owner-only file privacy.
                os.chown(path, -1, worker.pw_gid, follow_symlinks=False)
                os.chmod(path, mode | ((mode & 0o700) >> 3))


if __name__ == "__main__":
    try:
        prepare(sys.argv[1:])
    except (OSError, ValueError, KeyError):
        print("development workspace access preparation failed", file=sys.stderr)
        sys.exit(1)
