#!/usr/bin/env python3
"""Extract a reviewed Git tar without following archive links."""

from __future__ import annotations

import os
import stat
import sys
import tarfile
from pathlib import PurePosixPath


class ArchiveError(Exception):
    pass


def archive_path(name: str) -> tuple[str, ...]:
    if not name or name.startswith("/"):
        raise ArchiveError("release archive contains an absolute or empty path")
    if name in (".", "./"):
        return ()
    path = PurePosixPath(name)
    parts = path.parts
    if ".." in parts:
        raise ArchiveError("release archive contains path traversal")
    return parts


def assert_inside(root: str, target: str) -> None:
    try:
        if os.path.commonpath((root, os.path.realpath(target))) != root:
            raise ArchiveError("release archive escapes its extraction root")
    except ValueError as error:
        raise ArchiveError("release archive escapes its extraction root") from error


def ensure_parent(root: str, parts: tuple[str, ...]) -> str:
    current = root
    for part in parts:
        current = os.path.join(current, part)
        try:
            entry = os.lstat(current)
        except FileNotFoundError:
            os.mkdir(current, 0o700)
            continue
        if stat.S_ISLNK(entry.st_mode):
            raise ArchiveError("release archive contains a symbolic link in a parent path")
        if not stat.S_ISDIR(entry.st_mode):
            raise ArchiveError("release archive contains a non-directory parent path")
    assert_inside(root, current)
    return current


def extract(archive_pathname: str, destination: str) -> None:
    destination = os.path.abspath(destination)
    try:
        root_status = os.lstat(destination)
    except OSError as error:
        raise ArchiveError("release extraction root is unavailable") from error
    if not stat.S_ISDIR(root_status.st_mode) or stat.S_ISLNK(root_status.st_mode):
        raise ArchiveError("release extraction root is not a directory")
    # A parent may be a platform alias such as macOS /var -> /private/var.
    # The final component was checked with lstat above, so resolving parents
    # does not permit a symlink extraction root.
    root = os.path.realpath(destination)

    seen: set[str] = set()
    with tarfile.open(archive_pathname, mode="r:*") as archive:
        for member in archive:
            parts = archive_path(member.name)
            normalized = "/".join(parts)
            if normalized in seen:
                raise ArchiveError(f"release archive contains duplicate path {normalized}")
            seen.add(normalized)

            if member.issym():
                raise ArchiveError("release archive contains a symbolic link")
            if member.islnk():
                raise ArchiveError("release archive contains a hard link")
            if member.isdev() or member.isfifo():
                raise ArchiveError("release archive contains a special file")
            if not member.isdir() and not member.isfile():
                raise ArchiveError("release archive contains an unsupported entry")
            if not parts:
                if not member.isdir():
                    raise ArchiveError("release archive root must be a directory")
                continue

            if member.isdir():
                target = ensure_parent(root, parts)
                try:
                    status = os.lstat(target)
                except FileNotFoundError:
                    os.mkdir(target, 0o700)
                else:
                    if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
                        raise ArchiveError(f"release archive conflicts at {normalized}")
                os.chmod(target, member.mode & 0o755)
                continue

            parent = ensure_parent(root, parts[:-1])
            target = os.path.join(parent, parts[-1])
            assert_inside(root, target)
            try:
                descriptor = os.open(
                    target,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                    0o600,
                )
            except FileExistsError as error:
                raise ArchiveError(f"release archive conflicts at {normalized}") from error
            stream = archive.extractfile(member)
            if stream is None:
                os.close(descriptor)
                raise ArchiveError(f"release archive file cannot be read: {normalized}")
            try:
                written = 0
                with os.fdopen(descriptor, "wb") as output:
                    descriptor = -1
                    while True:
                        chunk = stream.read(1024 * 1024)
                        if not chunk:
                            break
                        output.write(chunk)
                        written += len(chunk)
                if written != member.size:
                    raise ArchiveError(f"release archive file is truncated: {normalized}")
            finally:
                stream.close()
                if descriptor != -1:
                    os.close(descriptor)
            os.chmod(target, member.mode & 0o755)


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: extract-cloud-staging-release.py ARCHIVE DESTINATION", file=sys.stderr)
        return 2
    try:
        extract(sys.argv[1], sys.argv[2])
    except (ArchiveError, OSError, tarfile.TarError, ValueError) as error:
        print(f"Chariox release archive extraction failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
