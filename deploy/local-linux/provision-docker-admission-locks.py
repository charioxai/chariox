#!/usr/bin/env python3
"""Provision shared empty locks without replacing legacy kernel lock inodes."""
import argparse
import os
from pathlib import Path
import stat

NAMES = ("chariox-docker-memory-admission.lock", "chariox-docker-disk-admission.lock")


def provision(root=Path("/"), dry_run=False):
    real_host = root == Path("/")
    if real_host and os.geteuid() != 0:
        raise PermissionError("run as root; all kernels sharing Docker must use these host-wide locks")
    owner = 0 if real_host else os.geteuid()  # --root is the installer fake-root test seam.
    parent = root / "tmp"
    if not parent.exists():
        if real_host:
            raise PermissionError("host /tmp is missing")
        if not dry_run:
            parent.mkdir(parents=True, mode=0o1777)
            parent.chmod(0o1777)
        else:
            print(f"would provision admission locks under {parent}")
            return
    parent = parent.resolve(strict=True)  # macOS /tmp -> /private/tmp.
    directory = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(directory)
        if metadata.st_uid != owner or (metadata.st_mode & 0o022 and not metadata.st_mode & stat.S_ISVTX):
            raise PermissionError("lock parent must be root-owned and sticky when writable")
        for name in NAMES:
            try:
                fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
            except FileNotFoundError:
                if dry_run:
                    print(f"would create {parent / name} mode 0444")
                    continue
                try:
                    fd = os.open(name, os.O_RDONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o444, dir_fd=directory)
                except FileExistsError:
                    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
            try:
                metadata = os.fstat(fd)
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != owner or metadata.st_nlink != 1 or metadata.st_size != 0 or metadata.st_mode & 0o022:
                    raise PermissionError(f"refusing unsafe legacy lock {parent / name}; stop all kernels before administrator repair; never unlink a live lock")
                if stat.S_IMODE(metadata.st_mode) != 0o444:
                    if not dry_run:
                        os.fchmod(fd, 0o444)
                    print(f"{'would set' if dry_run else 'set'} {parent / name} mode 0444 in place")
                else:
                    print(f"unchanged {parent / name}")
            finally:
                os.close(fd)
    finally:
        os.close(directory)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("/"), help="installer fake-root test prefix")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    provision(args.root, args.dry_run)
