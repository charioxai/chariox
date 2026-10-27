#!/usr/bin/env python3
"""Own one managed update's scratch across process and machine interruption."""
from contextlib import contextmanager
import fcntl
import json
import os
import re
import shutil
import stat
import sys
from pathlib import Path

MARKER = ".release-update-owner"
LOCK = ".release-update-lock"


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


def private_file(path, uid, maximum=2048):
    value = path.lstat()
    if (not stat.S_ISREG(value.st_mode) or value.st_uid != uid
            or stat.S_IMODE(value.st_mode) != 0o600 or value.st_nlink != 1
            or value.st_size > maximum):
        raise ValueError("unsafe release update ownership marker or journal")
    return value


def mkdir_private(path):
    # The boundary must be private even if killed before a later chmod could run.
    previous = os.umask(0o077)
    try:
        path.mkdir(mode=0o700)
    finally:
        os.umask(previous)


def create_private_file(path, flags):
    previous = os.umask(0o077)
    try:
        return os.open(path, flags | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    finally:
        os.umask(previous)


@contextmanager
def locked(root, uid):
    lock = root / LOCK
    try:
        fd = create_private_file(lock, os.O_RDWR)
    except FileExistsError:
        private_file(lock, uid)
        fd = os.open(lock, os.O_RDWR | os.O_NOFOLLOW)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        private_file(lock, uid)
        yield
    finally:
        os.close(fd)


def journal_path(path):
    return path.parent / ("." + path.name + ".owner")


def publish(path, text, uid, temporary=None):
    # A killed write leaves only an exact private temporary file. The previous
    # published record remains authoritative until atomic replacement + fsync.
    temporary = temporary or path.with_name(path.name + ".tmp")
    if temporary.exists() or temporary.is_symlink():
        private_file(temporary, uid)
        temporary.unlink()
    fd = create_private_file(temporary, os.O_WRONLY)
    try:
        data = text.encode()
        while data:
            written = os.write(fd, data)
            if not written:
                raise OSError("release update ownership write made no progress")
            data = data[written:]
        os.fsync(fd)
    finally:
        os.close(fd)
    os.replace(temporary, path)
    sync_directory(path.parent)


def journal_value(path, owner=None):
    root = path.parent.stat()
    value = {"version": 1, "updateId": path.name, "root": str(path.parent),
             "rootDevice": root.st_dev, "rootInode": root.st_ino,
             "phase": "creating" if owner is None else "owned"}
    if owner is not None:
        value.update(device=owner.st_dev, inode=owner.st_ino)
    return value


def publish_journal(path, uid, owner=None):
    publish(journal_path(path), json.dumps(journal_value(path, owner)) + "\n", uid)


def read_journal(path, uid):
    journal = journal_path(path)
    if not journal.exists() and not journal.is_symlink():
        return None
    private_file(journal, uid)
    value = json.loads(journal.read_text())
    if not isinstance(value, dict) or value.get("phase") not in ("creating", "owned"):
        raise ValueError("invalid release update journal")
    expected = journal_value(path)
    if value["phase"] == "owned":
        if type(value.get("device")) is not int or type(value.get("inode")) is not int:
            raise ValueError("invalid release update journal inode")
        expected.update(phase="owned", device=value["device"], inode=value["inode"])
    if value != expected:
        raise ValueError("release update journal identity or root mismatch")
    return value


def check_marker(path, update_id, uid):
    private_file(path, uid, maximum=256)
    if path.read_text() != update_id + "\n":
        raise ValueError("release update ownership mismatch")


def check_tree(path, owner):
    if owner.st_dev != path.parent.stat().st_dev or os.path.ismount(path):
        raise ValueError("mounted release update boundary")
    if stat.S_IMODE(owner.st_mode) != 0o700:
        raise ValueError("release update boundary must be private")
    mounts = Path("/proc/self/mountinfo")
    if mounts.exists():
        for line in mounts.read_text().splitlines():
            fields = line.split()
            if len(fields) < 5:
                raise ValueError("invalid mount inventory")
            mount = re.sub(r"\\([0-7]{3})", lambda match: chr(int(match[1], 8)), fields[4])
            if mount == str(path) or mount.startswith(str(path) + "/"):
                raise ValueError("mounted content in release update scratch")
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


def discard_journal(path, uid):
    # These exact per-ID names are reserved helper sidecars, never sibling trees.
    # The durable external proof outlives all boundary deletion and its fsync.
    for item in [journal_path(path).with_name(journal_path(path).name + ".tmp"),
                 path.parent / ("." + path.name + ".marker.tmp"), journal_path(path)]:
        if item.exists() or item.is_symlink():
            private_file(item, uid)
            item.unlink()
    sync_directory(path.parent)


def cleanup_locked(path, uid):
    value = read_journal(path, uid)
    if not path.exists() and not path.is_symlink():
        sync_directory(path.parent)
        discard_journal(path, uid)
        return
    owner = directory(path, uid)
    check_tree(path, owner)
    marker = path / MARKER
    if value is None:
        # Migrate an intact legacy boundary before removing anything in it.
        check_marker(marker, path.name, uid)
        publish_journal(path, uid, owner)
    elif value["phase"] == "creating":
        # Only mkdir can have happened before inode publication; no content was
        # admitted yet. An intent never grants ownership of unknown files.
        if any(path.iterdir()):
            raise ValueError("unpublished release update boundary is not empty")
    else:
        if owner.st_dev != value["device"] or owner.st_ino != value["inode"]:
            raise ValueError("release update boundary inode mismatch")
        if marker.exists() or marker.is_symlink():
            check_marker(marker, path.name, uid)
    shutil.rmtree(path)
    sync_directory(path.parent)
    discard_journal(path, uid)


def cleanup(root, update_id, expected_uid=0):
    path = boundary(root, update_id, expected_uid)
    if not path.parent.exists():
        return
    with locked(path.parent, expected_uid):
        cleanup_locked(path, expected_uid)


def prepare(root, update_id, expected_uid=0):
    path = boundary(root, update_id, expected_uid)
    if not path.parent.exists():
        try:
            mkdir_private(path.parent)
        except FileExistsError:
            pass  # A concurrent helper may have just established the root.
        directory(path.parent, expected_uid)
        sync_directory(path.parent.parent)
    with locked(path.parent, expected_uid):
        cleanup_locked(path, expected_uid)
        # Persist intent only after refusing every pre-existing unknown boundary.
        publish_journal(path, expected_uid)
        mkdir_private(path)
        sync_directory(path.parent)
        publish_journal(path, expected_uid, directory(path, expected_uid))
        # Publish the legacy marker atomically, with its temporary outside the
        # boundary so an interruption never leaves a partial marker inside it.
        temporary_marker = path.parent / ("." + path.name + ".marker.tmp")
        publish(path / MARKER, update_id + "\n", expected_uid, temporary=temporary_marker)
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
