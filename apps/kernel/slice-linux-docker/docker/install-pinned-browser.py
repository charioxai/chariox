#!/usr/bin/env python3
"""Install the host browser's verified pin into an amd64 slice image."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import urllib.request
import zipfile


def install(lock_path, destination):
    lock = json.loads(Path(lock_path).read_text())
    if lock["schema"] != "chariox.kernel-browser-chromium.v1":
        raise ValueError("unsupported browser lock")
    pin = lock["linux64"]
    root = Path(destination)
    with tempfile.TemporaryDirectory() as temporary:
        archive = Path(temporary) / "browser.zip"
        with urllib.request.urlopen(pin["url"], timeout=120) as response, archive.open("wb") as output:
            shutil.copyfileobj(response, output)
        with archive.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if archive.stat().st_size != pin["size"] or digest != pin["sha256"]:
            raise ValueError("browser archive does not match the approved pin")
        target = root / "chromium" / lock["version"]
        with zipfile.ZipFile(archive) as bundle:
            for entry in bundle.infolist():
                if entry.is_dir():
                    continue
                path = target / entry.filename
                if not path.resolve().is_relative_to(target.resolve()):
                    raise ValueError("unsafe browser archive path")
                path.parent.mkdir(parents=True, exist_ok=True)
                with bundle.open(entry) as source, path.open("wb") as output:
                    shutil.copyfileobj(source, output)
                path.chmod(0o755 if (entry.external_attr >> 16) & 0o111 else 0o644)
        executable = target / pin["executable"]
        if not os.access(executable, os.X_OK):
            raise ValueError("browser executable is missing")
        # CfT ships its helper without setuid or Chrome's expected filename.
        # The verified helper lives in the root-owned, read-only image tree.
        sandbox = executable.with_name("chrome_sandbox")
        if not sandbox.is_file() or sandbox.stat().st_uid != 0 or not os.access(sandbox, os.X_OK):
            raise ValueError("root-owned browser sandbox helper is missing")
        sandbox.chmod(0o4755)
        executable.with_name("chrome-sandbox").symlink_to(sandbox.name)
        (root / "chrome").symlink_to(executable)


if __name__ == "__main__":
    install(*sys.argv[1:])
