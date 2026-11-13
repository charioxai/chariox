#!/usr/bin/env python3
"""Validate and extract a bounded release archive before signature verification."""
import gzip
import os
import posixpath
import shutil
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Limits:
    archive_bytes: int = 2 * 1024**3
    tar_bytes: int = 8 * 1024**3
    file_bytes: int = 8 * 1024**3
    members: int = 50000
    metadata_bytes: int = 65536
    metadata_total_bytes: int = 16 * 1024**2
    reserve_bytes: int = 2 * 1024**3
    reserve_inodes: int = 4096


class UnsafeArchive(ValueError):
    pass


class ReleaseMember(tarfile.TarInfo):
    _limits = Limits()

    def _charge_metadata(self, archive):
        if self.size < 0 or self.size > self._limits.metadata_bytes:
            raise UnsafeArchive("oversized archive metadata")
        total = getattr(archive, "_release_metadata_bytes", 0) + tarfile.BLOCKSIZE + self._block(self.size)
        if total > self._limits.metadata_total_bytes:
            raise UnsafeArchive("aggregate archive metadata exceeds limit")
        archive._release_metadata_bytes = total

    def _proc_pax(self, archive):
        if self.type == tarfile.XGLTYPE:
            raise UnsafeArchive("global archive metadata is unsupported")
        self._charge_metadata(archive)
        return super()._proc_pax(archive)

    def _proc_gnulong(self, archive):
        self._charge_metadata(archive)
        return super()._proc_gnulong(archive)

    def _proc_sparse(self, archive):
        raise UnsafeArchive("sparse archive member")

    def _proc_gnusparse_00(self, *args):
        raise UnsafeArchive("sparse archive member")

    _proc_gnusparse_01 = _proc_gnusparse_00
    _proc_gnusparse_10 = _proc_gnusparse_00


def path_name(name):
    if "\x00" in name or len(name.encode("utf-8", "surrogateescape")) > 4096:
        raise UnsafeArchive("invalid archive path")
    if name.startswith("/") or ".." in name.split("/"):
        raise UnsafeArchive("archive path escapes destination")
    return posixpath.normpath(name)


def space_available(path, bytes_needed, inodes_needed, limits):
    stat = os.statvfs(path)
    if stat.f_bavail * stat.f_frsize < bytes_needed + limits.reserve_bytes:
        raise UnsafeArchive("insufficient extraction disk headroom")
    if stat.f_favail < inodes_needed + limits.reserve_inodes:
        raise UnsafeArchive("insufficient extraction inode headroom")


def validate(archive, limits, block_size):
    members = {}
    allocated = 0
    logical = 0
    count = 0
    for member in archive:
        count += 1
        if count > limits.members:
            raise UnsafeArchive("too many archive members")
        name = path_name(member.name)
        if name == ".":
            if not member.isdir():
                raise UnsafeArchive("archive root must be a directory")
            continue
        if name in members:
            raise UnsafeArchive("duplicate archive path")
        if member.sparse is not None or any(key.startswith("GNU.sparse") for key in member.pax_headers):
            raise UnsafeArchive("sparse archive member")
        if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
            raise UnsafeArchive("unsupported archive member type")
        if member.size < 0 or (not member.isfile() and member.size):
            raise UnsafeArchive("invalid archive member size")
        logical += member.size
        allocated += ((member.size + block_size - 1) // block_size) * block_size + block_size
        if logical > limits.file_bytes or allocated > limits.file_bytes:
            raise UnsafeArchive("expanded archive exceeds storage limit")
        members[name] = member
    for name, member in members.items():
        parent = posixpath.dirname(name)
        while parent:
            if parent not in members or not members[parent].isdir():
                raise UnsafeArchive("archive parent must be an explicit directory")
            parent = posixpath.dirname(parent)
        if member.issym():
            target = member.linkname
            if not target or target.startswith("/") or "\x00" in target:
                raise UnsafeArchive("unsafe symlink target")
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
            if resolved == ".." or resolved.startswith("../"):
                raise UnsafeArchive("symlink escapes destination")
            # Links never become extraction parents. Requiring a real member as
            # target also prevents chains from escaping or hiding missing files.
            if resolved not in members or not (members[resolved].isfile() or members[resolved].isdir()):
                raise UnsafeArchive("symlink target must be a regular file or directory")
        if member.islnk():
            target = path_name(member.linkname)
            if target not in members or not members[target].isfile():
                raise UnsafeArchive("hardlink target must be a regular file")
            # cp -RP in the upgrader may expand a hardlink into another file.
            size = members[target].size
            allocated += ((size + block_size - 1) // block_size) * block_size
            if allocated > limits.file_bytes:
                raise UnsafeArchive("expanded hardlinks exceed storage limit")
    return members, allocated


def extract_release(source, destination, limits=Limits(), publication=None):
    destination = Path(destination)
    parent = destination.parent
    if not parent.is_dir() or destination.exists() or destination.is_symlink():
        raise UnsafeArchive("destination must be absent in an existing directory")
    publication = Path(publication) if publication is not None else None
    if publication is not None and not publication.is_dir():
        raise UnsafeArchive("publication destination must be an existing directory")
    separate_publication = publication is not None and os.stat(publication).st_dev != os.stat(parent).st_dev
    block_size = os.statvfs(parent).f_frsize
    if separate_publication:
        block_size = max(block_size, os.statvfs(publication).f_frsize)

    class BoundedReleaseMember(ReleaseMember):
        _limits = limits

    # Owned scratch is private and never placed below a member-controlled path.
    with tempfile.TemporaryDirectory(prefix=".release-extract-", dir=parent) as scratch:
        scratch = Path(scratch)
        tar_path = scratch / "archive.tar"
        staging = scratch / "tree"
        space_available(parent, 0, 3, limits)
        with open(source, "rb") as compressed:
            if os.fstat(compressed.fileno()).st_size > limits.archive_bytes:
                raise UnsafeArchive("compressed archive exceeds limit")
            with gzip.GzipFile(fileobj=compressed) as stream, open(tar_path, "xb") as output:
                total = 0
                while True:
                    chunk = stream.read(1024 * 1024)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > limits.tar_bytes:
                        raise UnsafeArchive("tar stream exceeds limit")
                    space_available(parent, len(chunk), 0, limits)
                    output.write(chunk)
        with tarfile.open(tar_path, mode="r:", tarinfo=BoundedReleaseMember) as archive:
            members, allocated = validate(archive, limits, block_size)
            # upgrade-image stages and publishes additional copies. Admit all
            # three copies while retaining recovery space and tar scratch.
            space_available(parent, allocated * 3, len(members) * 3 + 1, limits)
            if separate_publication:
                space_available(publication, allocated, len(members) + 1, limits)
            staging.mkdir(mode=0o700)
            for name, member in sorted(members.items(), key=lambda item: (item[0].count("/"), item[0])):
                path = staging / name
                if member.isdir():
                    path.mkdir(mode=0o755)
                elif member.isfile():
                    space_available(parent, member.size, 1, limits)
                    with archive.extractfile(member) as content, open(path, "xb") as output:
                        shutil.copyfileobj(content, output, length=1024 * 1024)
                    path.chmod(member.mode & 0o777)
            for name, member in members.items():
                if member.issym():
                    os.symlink(member.linkname, staging / name)
                elif member.islnk():
                    os.link(staging / path_name(member.linkname), staging / name)
            # Restore signed directory modes after all children and links exist;
            # mkdir alone applies the caller's umask to those modes.
            for name, member in sorted(members.items(), key=lambda item: item[0].count("/"), reverse=True):
                if member.isdir():
                    (staging / name).chmod(member.mode & 0o777)
            if destination.exists() or destination.is_symlink():
                raise UnsafeArchive("destination appeared during extraction")
            staging.rename(destination)


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (3, 4):
            raise UnsafeArchive("usage: extract-release.py ARCHIVE.tar.gz DESTINATION [PUBLICATION_DIRECTORY]")
        extract_release(sys.argv[1], sys.argv[2], publication=sys.argv[3] if len(sys.argv) == 4 else None)
    except (UnsafeArchive, OSError, tarfile.TarError, EOFError) as error:
        print(f"release extraction refused: {error}", file=sys.stderr)
        sys.exit(1)
