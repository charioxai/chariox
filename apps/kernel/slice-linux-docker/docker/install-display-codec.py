#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: install Cisco's pinned public codec in kernel-owned state."""
import argparse
import bz2
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import tempfile
import urllib.request

MAX_COMPRESSED = 4 * 1024 * 1024
MAX_LIBRARY = 16 * 1024 * 1024

def checked_library(data, pin):
    if len(data) > MAX_COMPRESSED or hashlib.sha256(data).hexdigest() != pin["compressed_sha256"]:
        raise ValueError("MP-11: compressed display codec hash mismatch")
    decoder = bz2.BZ2Decompressor()
    library = decoder.decompress(data, max_length=MAX_LIBRARY + 1)
    if len(library) > MAX_LIBRARY or not decoder.eof or decoder.unused_data:
        raise ValueError("MP-11: display codec decompression bound")
    if hashlib.sha256(library).hexdigest() != pin["library_sha256"]:
        raise ValueError("MP-11: display codec library hash mismatch")
    return library

def install(root, pin):
    if not root.is_absolute():
        raise ValueError("MP-11: absolute display codec cache required")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = root.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
        raise ValueError("MP-11: display codec cache is not kernel-owned")
    lock = os.open(root / ".install.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(lock)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_nlink != 1 or info.st_mode & 0o077:
            raise ValueError("MP-11: display codec lock ownership")
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = root / "libopenh264.so.8"
        try:
            fd = os.open(destination, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        except FileNotFoundError:
            fd = None
        if fd is not None:
            with os.fdopen(fd, "rb") as source:
                info = os.fstat(source.fileno())
                if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_nlink != 1 or info.st_size > MAX_LIBRARY or info.st_mode & 0o022:
                    raise ValueError("MP-11: display codec library ownership")
                if hashlib.sha256(source.read(MAX_LIBRARY + 1)).hexdigest() == pin["library_sha256"]:
                    return destination
            # This exact public cache file may be repaired only by a fresh,
            # doubly verified Cisco download. It is never loaded while invalid.
        url = pin["url"]
        if not url.startswith("https://ciscobinary.openh264.org/libopenh264-") or not url.endswith(".so.bz2"):
            raise ValueError("MP-11: Cisco HTTPS display codec URL required")
        with urllib.request.urlopen(url, timeout=30) as response:
            if not response.geturl().startswith("https://ciscobinary.openh264.org/"):
                raise ValueError("MP-11: display codec download origin changed")
            data = response.read(MAX_COMPRESSED + 1)
        library = checked_library(data, pin)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(prefix=".codec-", dir=root, delete=False) as output:
                temporary = Path(output.name)
                output.write(library)
                output.flush()
                os.fsync(output.fileno())
                os.fchmod(output.fileno(), 0o644)
            os.replace(temporary, destination)
            temporary = None
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        return destination
    finally:
        os.close(lock)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    arch = {"x86_64": "amd64", "aarch64": "arm64"}.get(platform.machine())
    pin = json.loads(Path(__file__).with_name("display-codec-pin.json").read_text())["targets"].get(arch)
    if pin is None:
        raise ValueError("MP-08: unsupported display codec architecture")
    print(install(args.root, pin))

if __name__ == "__main__":
    main()
