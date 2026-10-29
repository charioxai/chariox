#!/usr/bin/env python3
"""Install only checksum-pinned official toolchains into an explicit directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import sys
import tarfile
import tempfile
import urllib.request

SDK = Path(__file__).resolve().parents[1]
LOCK = SDK / "toolchain.lock.json"


def lock():
    return json.loads(LOCK.read_text())


def assets():
    key = f"{platform.system()}-{platform.machine()}"
    pins = lock()["platforms"]
    if key not in pins:
        raise ValueError(f"unsupported build host {key}; add reviewed toolchain checksums first")
    return pins[key]


def tools(root):
    pins = assets()
    if (root / "toolchain.lock.json").read_bytes() != LOCK.read_bytes():
        raise ValueError("toolchain pins changed; install into a fresh directory")
    return root / pins[0]["directory"] / "wit-bindgen", root / pins[1]["directory"] / "bin" / "wasm32-wasip2-clang"


def install(root):
    if root.exists() and any(root.iterdir()):
        raise ValueError("install destination must be empty; choose a fresh directory")
    root.mkdir(parents=True, exist_ok=True)
    for pin in assets():
        with tempfile.TemporaryFile(dir=root) as archive:
            with urllib.request.urlopen(pin["url"], timeout=120) as response:
                digest = hashlib.sha256()
                while chunk := response.read(1024 * 1024):
                    digest.update(chunk)
                    archive.write(chunk)
            if digest.hexdigest() != pin["sha256"]:
                raise ValueError(f"checksum mismatch for {pin['url']}; remove this incomplete install and retry")
            archive.seek(0)
            # Python's data filter refuses escaping links and special files.
            with tarfile.open(fileobj=archive, mode="r:gz") as source:
                source.extractall(root, filter="data")
        print(f"verified {pin['directory']}: {pin['sha256']}", flush=True)
    (root / "toolchain.lock.json").write_bytes(LOCK.read_bytes())


def selected_root(value=None):
    value = value or os.environ.get("CHARIOX_C_TOOLCHAIN")
    if not value:
        raise ValueError("set CHARIOX_C_TOOLCHAIN to a directory installed by scripts/toolchain.py")
    return Path(value).resolve()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        install(args.directory.resolve())
    except (OSError, ValueError) as error:
        sys.exit(str(error))
