#!/usr/bin/env python3
"""Validate decompressed tar metadata before restore; never print entry contents."""
import posixpath
import sys
import tarfile

PRIVATE_PATHS = (
    ".codex", ".claude", ".claude.json", ".ssh", ".gnupg", ".config/gh",
    ".local/share/opencode", ".local/state/chariox", ".local/share/pki/nssdb/key4.db",
    ".chariox/kernels", ".chariox/daemon", ".chariox/state", ".chariox/provider-home", ".chariox/keys",
)


def safe_path(name):
    if name.startswith("/") or ".." in name.split("/") or "\x00" in name:
        raise ValueError("unsupported archive path")
    path = posixpath.normpath(name)
    if any(path == root or path.startswith(root + "/") for root in PRIVATE_PATHS):
        raise ValueError("archive contains unsupported credential layout")
    return path


def validate_members(members):
    seen = set()
    for member in members:
        path = safe_path(member.name)
        if path in seen:
            raise ValueError("duplicate archive member")
        seen.add(path)
        if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
            raise ValueError("unsupported archive member type")
        if member.issym():
            safe_path(member.linkname)
            safe_path(posixpath.join(posixpath.dirname(path), member.linkname))
        elif member.islnk():
            safe_path(member.linkname)
        if member.mode & 0o6000:
            raise ValueError("unsupported archive file permissions")


if __name__ == "__main__":
    try:
        with tarfile.open(fileobj=sys.stdin.buffer, mode="r|") as archive:
            validate_members(archive)
    except (ValueError, tarfile.TarError, OSError):
        print("Slice restore refused an unsupported archive layout; existing private state is preserved", file=sys.stderr)
        sys.exit(1)
