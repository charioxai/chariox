#!/usr/bin/env python3
"""Read durable Apps footprints, including initialized tables even with no rows."""
import json
import pathlib
import sqlite3
import stat
import sys


def safe_metadata(path, root):
    for item in [path, *path.parents]:
        if item == root.parent:
            break
        metadata = item.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise ValueError("redirected state path")
    return path.lstat()


def has_app_state(root):
    homes = [root / "home/chariox/.chariox", root / "var/lib/chariox/home",
             root / "var/lib/chariox/home/.chariox"]
    databases = [home / "state/kernel.db" for home in homes]
    enrollment = root / "etc/chariox/app-storage.json"
    if enrollment.exists() or enrollment.is_symlink():
        if not stat.S_ISREG(safe_metadata(enrollment, root).st_mode):
            raise ValueError("invalid storage enrollment")
        for owner in json.loads(enrollment.read_text()).get("owners", []):
            for name in owner.get("kernel_database_paths", []):
                path = pathlib.PurePosixPath(name)
                if not path.is_absolute() or ".." in path.parts:
                    raise ValueError("invalid enrolled database path")
                databases.append(root / str(path).lstrip("/"))
    roots = [root / "var/lib/chariox-app-storage"]
    for database in set(databases):
        parent = database.parent
        if parent.exists():
            safe_metadata(parent, root)
            roots.extend(item for item in parent.iterdir() if item.name.startswith("app-releases-")
                         or item.name in ("app-storage", "app-restores", "app-snapshots"))
        if database.exists() or database.is_symlink():
            if not stat.S_ISREG(safe_metadata(database, root).st_mode):
                raise ValueError("invalid kernel database")
            with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=5) as connection:
                # Phase 1 initialization writes these tables before any App is
                # installed. Initialized tables are part of the state boundary.
                if connection.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name GLOB 'app_*' LIMIT 1").fetchone():
                    return True
    for directory in roots:
        if directory.exists() or directory.is_symlink():
            if not stat.S_ISDIR(safe_metadata(directory, root).st_mode):
                raise ValueError("invalid App storage root")
            if next(directory.iterdir(), None) is not None:
                return True
    return False


if __name__ == "__main__":
    try:
        print("present" if has_app_state(pathlib.Path(sys.argv[1] or "/").absolute()) else "absent")
    except (OSError, ValueError, sqlite3.Error, KeyError, TypeError, AttributeError):
        # Never infer absence from unreadable, redirected or corrupt state.
        print("present")
        print("Could not prove App state absent; the Apps rollback override is required.", file=sys.stderr)
