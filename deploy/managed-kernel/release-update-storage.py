#!/usr/bin/env python3
"""Own one managed update's scratch across process and machine interruption."""
import os
import re
import shutil
import stat
import sys
from pathlib import Path

MARKER = ".release-update-owner"


def directory(path, uid):
    value = path.lstat()
    if not stat.S_ISDIR(value.st_mode) or value.st_uid != uid or value.st_mode & 0o022:
        raise ValueError("unsafe release update directory")
    return value


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def boundary(root, update_id, expected_uid):
    if not re.fullmatch(r"managed_release_update_[a-f0-9-]{36}", update_id):
        raise ValueError("invalid release update identity")
    root = Path(root)
    if not root.is_absolute():
        raise ValueError("release update root must be absolute")
    # Reject aliases through any ancestor before inspecting or removing files.
    if root.resolve() != root:
        raise ValueError("release update root must not contain symlinks")
    if root.exists() or root.is_symlink():
        directory(root, expected_uid)
    return root / update_id


def cleanup(root, update_id, expected_uid=0):
    path = boundary(root, update_id, expected_uid)
    if not path.exists() and not path.is_symlink():
        return
    owner = directory(path, expected_uid)
    if owner.st_dev != path.parent.stat().st_dev or os.path.ismount(path):
        raise ValueError("mounted release update boundary")
    mounts = Path("/proc/self/mountinfo")
    if mounts.exists():
        for line in mounts.read_text().splitlines():
            fields = line.split()
            if len(fields) < 5:
                raise ValueError("invalid mount inventory")
            mount = re.sub(r"\\([0-7]{3})", lambda match: chr(int(match[1], 8)), fields[4])
            if mount == str(path) or mount.startswith(str(path) + "/"):
                raise ValueError("mounted content in release update scratch")
    if stat.S_IMODE(owner.st_mode) != 0o700:
        raise ValueError("release update boundary must be private")
    marker = path / MARKER
    info = marker.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != expected_uid or info.st_mode & 0o077 or info.st_size > 256:
        raise ValueError("unsafe release update ownership marker")
    if marker.read_text() != update_id + "\n":
        raise ValueError("release update ownership mismatch")
    # Inventory the exact boundary. Links are removed as links, never followed;
    # mounted trees and special files are not owned release scratch.
    for parent, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            item = Path(parent) / name
            value = item.lstat()
            if value.st_dev != owner.st_dev or os.path.ismount(item):
                raise ValueError("mounted content in release update scratch")
            if not (stat.S_ISREG(value.st_mode) or stat.S_ISDIR(value.st_mode) or stat.S_ISLNK(value.st_mode)):
                raise ValueError("special file in release update scratch")
    shutil.rmtree(path)
    sync_directory(path.parent)


def prepare(root, update_id, expected_uid=0):
    path = boundary(root, update_id, expected_uid)
    if not path.parent.exists():
        path.parent.mkdir(mode=0o700)
        path.parent.chmod(0o700)
        directory(path.parent, expected_uid)
        sync_directory(path.parent.parent)
    cleanup(root, update_id, expected_uid)
    path.mkdir(mode=0o700)
    path.chmod(0o700)
    # Persist ownership before any large extraction writes can occur.
    fd = os.open(path / MARKER, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.fchmod(fd, 0o600)
        os.write(fd, (update_id + "\n").encode())
        os.fsync(fd)
    finally:
        os.close(fd)
    sync_directory(path)
    sync_directory(path.parent)
    return path


if __name__ == "__main__":
    try:
        if len(sys.argv) != 4 or sys.argv[1] not in ("prepare", "cleanup") or os.geteuid() != 0:
            raise ValueError("usage as root: release-update-storage.py prepare|cleanup ROOT UPDATE_ID")
        {"prepare": prepare, "cleanup": cleanup}[sys.argv[1]](sys.argv[2], sys.argv[3])
    except (OSError, ValueError) as error:
        print(f"release update storage refused: {error}", file=sys.stderr)
        sys.exit(1)
