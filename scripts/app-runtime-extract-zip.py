#!/usr/bin/env python3
"""Extract one reviewed native CI artifact; never enroll or execute it.

Inputs and scratch remain under exclusive builder control. These path-based
operations are not the installer's hostile/concurrent filesystem boundary.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import sys
import zipfile

FILES = {"libnode.so.137", "libchariox-app-runtime.so", "NODE-LICENSE", "artifact-manifest.json"}
SMALL = {"NODE-LICENSE", "artifact-manifest.json"}
MAX_TOTAL = 512 * 1024 * 1024
MAX_SMALL = 256 * 1024
MAX_DIRECTORY = 64 * 1024


def require(value):
    if not value:
        raise ValueError("invalid runtime artifact ZIP")


def read_at(file, offset, length):
    require(offset >= 0 and 0 <= length <= MAX_DIRECTORY)
    file.seek(offset)
    data = file.read(length)
    require(len(data) == length)
    return data


def extra_fields(data):
    require(len(data) <= 4096)
    while data:
        require(len(data) >= 4)
        kind, length = struct.unpack_from("<HH", data)
        require(kind != 1 and length <= len(data) - 4)  # No ZIP64.
        data = data[4 + length:]


def directory(file, size):
    # Admit the central directory before ZipFile can allocate from its counts.
    require(22 <= size <= MAX_TOTAL + 1024 * 1024)
    end = struct.unpack("<4s4H2IH", read_at(file, size - 22, 22))
    signature, disk, central_disk, disk_count, count, length, offset, comment = end
    require(signature == b"PK\x05\x06" and disk == central_disk == comment == 0)
    require(disk_count == count == len(FILES) and 0 < length <= MAX_DIRECTORY)
    require(offset + length == size - 22)
    data = read_at(file, offset, length)
    members = []
    cursor = 0
    names = set()
    total = 0
    while cursor < len(data):
        require(len(members) < len(FILES) and cursor + 46 <= len(data))
        header = struct.unpack_from("<4s6H3I5H2I", data, cursor)
        sig, made, needed, flags, method, _time, _date, crc, packed, plain, name_len, extra_len, note_len, start_disk, _attrs, attrs, local = header
        require(sig == b"PK\x01\x02" and needed <= 20 and method in (0, 8))
        require(flags & ~(0x800 | 0x8 | 0x6) == 0 and (method == 8 or flags & 0x6 == 0))
        require(start_disk == note_len == 0 and 0 < name_len <= 64 and extra_len <= 4096)
        finish = cursor + 46 + name_len + extra_len
        require(finish <= len(data))
        encoded = data[cursor + 46:cursor + 46 + name_len]
        name = encoded.decode("ascii")
        require(name in FILES and name not in names)
        names.add(name)
        require(not attrs & 0x10 and stat.S_IFMT(attrs >> 16) in (0, stat.S_IFREG))
        require(made >> 8 in (0, 3))  # DOS or Unix regular-file metadata.
        extra_fields(data[cursor + 46 + name_len:finish])
        require(0 < plain <= (MAX_SMALL if name in SMALL else MAX_TOTAL) and 0 <= packed <= MAX_TOTAL)
        total += plain
        require(total <= MAX_TOTAL)
        members.append((local, name, encoded, needed, flags, method, crc, packed, plain))
        cursor = finish
    require(names == FILES)
    # No hidden local entries, overlaps, prefix executable, or trailing payload.
    cursor = 0
    for local, _name, encoded, needed, flags, method, crc, packed, plain in sorted(members):
        require(local == cursor and local + 30 <= offset)
        local_header = struct.unpack("<4s5H3I2H", read_at(file, local, 30))
        sig, version, local_flags, local_method, _time, _date, local_crc, local_packed, local_plain, name_len, extra_len = local_header
        require(sig == b"PK\x03\x04" and (version, local_flags, local_method) == (needed, flags, method))
        require(name_len == len(encoded) and extra_len <= 4096)
        require(local + 30 + name_len + extra_len <= offset)
        tail = read_at(file, local + 30, name_len + extra_len)
        require(tail[:name_len] == encoded)
        extra_fields(tail[name_len:])
        if flags & 8:
            require(local_crc in (0, crc) and local_packed in (0, packed) and local_plain in (0, plain))
        else:
            require((local_crc, local_packed, local_plain) == (crc, packed, plain))
        cursor = local + 30 + name_len + extra_len + packed
        require(cursor <= offset)
        if flags & 8:
            require(cursor + 12 <= offset)
            descriptor = read_at(file, cursor, min(16, offset - cursor))
            skip = 4 if descriptor[:4] == b"PK\x07\x08" else 0
            require(len(descriptor) >= skip + 12)
            require(struct.unpack_from("<3I", descriptor, skip) == (crc, packed, plain))
            cursor += skip + 12
    require(cursor == offset)
    return {member[1]: member[-1] for member in members}


def builder_parent(parent, private):
    require(parent.is_absolute() and parent.resolve(strict=True) == parent)
    current = parent
    while True:
        info = current.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid in (0, os.getuid()))
        sticky_root = info.st_uid == 0 and info.st_mode & stat.S_ISVTX
        require(not info.st_mode & 0o022 or sticky_root)
        require(not os.path.lexists(current / ".git"))
        if current == current.parent:
            break
        current = current.parent
    if private:
        info = parent.stat()
        require(info.st_uid == os.getuid() and not info.st_mode & 0o077)


def extract(archive, destination, expected_sha256):
    archive, destination = Path(archive), Path(destination)
    require(re.fullmatch(r"[a-f0-9]{64}", expected_sha256) is not None)
    require(archive.is_absolute() and destination.is_absolute() and archive.resolve(strict=True) == archive)
    builder_parent(archive.parent, False)
    builder_parent(destination.parent, True)
    require(not os.path.lexists(destination))
    descriptor = os.open(archive, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    owned = None
    try:
        with os.fdopen(descriptor, "rb") as file:
            info = os.fstat(file.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.getuid())
            require(not info.st_mode & 0o022 and info.st_size <= MAX_TOTAL + 1024 * 1024)
            digest = hashlib.sha256()
            read = 0
            while chunk := file.read(65536):
                read += len(chunk)
                require(read <= MAX_TOTAL + 1024 * 1024)
                digest.update(chunk)
            require(read == info.st_size and digest.hexdigest() == expected_sha256)
            expected = directory(file, info.st_size)
            destination.mkdir(mode=0o700)
            owned = destination.stat()
            with zipfile.ZipFile(file) as zipped:
                require(set(zipped.namelist()) == FILES and len(zipped.infolist()) == len(FILES))
                total = 0
                for name in sorted(FILES):
                    written = 0
                    fd = os.open(destination / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                    with os.fdopen(fd, "wb") as output, zipped.open(name) as member:
                        while chunk := member.read(65536):
                            written += len(chunk)
                            total += len(chunk)
                            require(written <= expected[name] and total <= MAX_TOTAL)
                            output.write(chunk)
                        require(written == expected[name])  # ZipExtFile also checks CRC at EOF.
                        output.flush()
                        os.fsync(output.fileno())
    except BaseException:
        if owned is not None:
            current = destination.lstat() if os.path.lexists(destination) else None
            if current and (current.st_dev, current.st_ino) == (owned.st_dev, owned.st_ino):
                shutil.rmtree(destination)
        raise


if __name__ == "__main__":
    try:
        class Arguments(argparse.ArgumentParser):
            def error(self, _message):
                raise ValueError("invalid arguments")
        parser = Arguments()
        parser.add_argument("--archive", required=True)
        parser.add_argument("--destination", required=True)
        parser.add_argument("--sha256", required=True)
        args = parser.parse_args()
        extract(args.archive, args.destination, args.sha256)
    except Exception:
        print("app_runtime_artifact_zip_rejected", file=sys.stderr)
        sys.exit(1)
