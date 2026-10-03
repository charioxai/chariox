#!/usr/bin/env python3
"""Install the shared Docker admission provisioner and its macOS boot daemon."""
import argparse
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

LABEL = "dev.chariox.docker-admission-locks"
HELPER = Path("usr/local/libexec/chariox/provision-docker-admission-locks.py")
DAEMON = Path(f"Library/LaunchDaemons/{LABEL}.plist")
SOURCE = Path(__file__).resolve().parents[1]


def trusted_directory(path, owner, boundary):
    if path != boundary:
        trusted_directory(path.parent, owner, boundary)
    if not path.exists() and not path.is_symlink():
        path.mkdir(mode=0o755)
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != owner or metadata.st_mode & 0o022:
        raise PermissionError(f"{path} must be an owned directory with no group/other writes or symlinks")


def install_file(source, target, mode, owner, boundary):
    trusted_directory(target.parent, owner, boundary)
    if target.exists() or target.is_symlink():
        metadata = target.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != owner or metadata.st_nlink != 1:
            raise PermissionError(f"refusing unsafe installed file {target}")
    fd, temporary = tempfile.mkstemp(prefix=".admission-", dir=target.parent)
    try:
        with os.fdopen(fd, "wb") as file:
            file.write(source.read_bytes())
            os.fchmod(file.fileno(), mode)
        os.replace(temporary, target)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def install(root=Path("/")):
    if not root.is_absolute():
        raise ValueError("--root must be absolute")
    real_host = root == Path("/")
    if real_host and (sys.platform != "darwin" or os.geteuid() != 0):
        raise PermissionError("run as root on macOS; --root is only for staged installer tests")
    owner = 0 if real_host else os.geteuid()
    install_file(SOURCE / "local-linux/provision-docker-admission-locks.py", root / HELPER, 0o555, owner, root)
    install_file(SOURCE / f"local-macos/{LABEL}.plist", root / DAEMON, 0o644, owner, root)
    # Provision synchronously before any kernel starts. Never repair or replace
    # an unsafe legacy lock, even when installing over an older kernel.
    subprocess.run(["/usr/bin/python3", str(root / HELPER), "--root", str(root)], check=True)
    if real_host:
        service = f"system/{LABEL}"
        if subprocess.run(["/bin/launchctl", "print", service], stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL).returncode == 0:
            subprocess.run(["/bin/launchctl", "bootout", service], check=True)
        subprocess.run(["/bin/launchctl", "enable", service], check=True)
        subprocess.run(["/bin/launchctl", "bootstrap", "system", str(root / DAEMON)], check=True)
    else:
        print("staged Docker admission boot daemon; host launchd unchanged")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("/"), help="installer fake-root prefix; never changes host launchd")
    install(parser.parse_args().root)
