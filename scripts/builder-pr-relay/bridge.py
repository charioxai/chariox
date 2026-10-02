"""Builder-side request reader and atomic PR feedback writer; holds no credentials."""
import json
import os
import re
import stat
import sys
from pathlib import Path

REPOS = ("chariox", "chariox-cloud")
BRANCH = re.compile(r"apps/p1-[A-Za-z0-9][A-Za-z0-9._/-]*\Z")
FOOTER = "🤖 Generated with [Claude Code](https://claude.com/claude-code)"


def valid_branch(value):
    return (isinstance(value, str) and BRANCH.fullmatch(value) is not None
            and all(x not in value for x in ("..", "//", "@{"))
            and all(p and not p.startswith(".") and not p.endswith((".lock", "."))
                    for p in value.split("/")))


def open_parent(root, relative, create=False):
    parts = Path(relative).parts
    if not parts or any(p in ("..", ".", "/") for p in parts):
        raise ValueError("invalid relative path")
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            if create:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                except FileExistsError:
                    pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd, parts[-1]
    except BaseException:
        os.close(fd)
        raise


def read_private(root, relative, limit):
    parent, name = open_parent(root, relative)
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(fd, "rb") as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise ValueError("request must be a regular file")
            data = stream.read(limit + 1)
            if len(data) > limit:
                raise ValueError("request exceeds limit")
            return data.decode("utf-8")
    finally:
        os.close(parent)


def collect(root):
    result = {repo: [] for repo in REPOS}
    errors = []
    for repo in REPOS:
        directory = root / repo
        if not directory.exists():
            continue
        for current, dirs, files in os.walk(directory, followlinks=False):
            dirs[:] = [d for d in dirs if not (Path(current) / d).is_symlink()]
            for name in sorted(files):
                if not name.endswith(".json"):
                    continue
                relative = (Path(current) / name).relative_to(root)
                branch = str(relative.relative_to(repo))[:-5]
                try:
                    if not valid_branch(branch):
                        raise ValueError("invalid branch")
                    request = json.loads(read_private(root, relative, 16384))
                    body_path = Path(request["body_file"])
                    body_relative = body_path.relative_to(root / repo)
                    body = read_private(root, Path(repo) / body_relative, 65536)
                    if not body.rstrip().endswith(FOOTER):
                        raise ValueError("missing PR footer")
                    result[repo].append({**request, "branch": branch, "body": body})
                except (OSError, ValueError, KeyError, TypeError):
                    errors.append({"repo": repo, "branch": branch, "error": "invalid request or body"})
    return {"requests": result, "errors": errors}


def write_mirror(root, relative, value):
    parent, name = open_parent(root, relative, create=True)
    temporary = f".{name}.{os.getpid()}.tmp"
    try:
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
        os.fsync(parent)
    finally:
        try:
            os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


def main():
    if sys.argv[1:] == ["read"]:
        print(json.dumps(collect(Path("/w/pr-requests"))))
    elif sys.argv[1:] == ["write"]:
        data = sys.stdin.buffer.read(16 * 1024 * 1024 + 1)
        if len(data) > 16 * 1024 * 1024:
            raise ValueError("mirror batch exceeds limit")
        records = json.loads(data)
        for entry in records:
            repo, key = entry["repo"], entry["key"]
            if repo not in REPOS:
                raise ValueError("invalid repository")
            if key.isdecimal() and int(key) > 0:
                relative = f"{repo}/{key}.json"
            elif key.startswith("branches/") and valid_branch(key[9:]):
                relative = f"{repo}/{key}.json"
            else:
                raise ValueError("invalid mirror key")
            write_mirror(Path("/w/pr-mirror"), relative, entry["value"])
    else:
        raise ValueError("use read or write")


if __name__ == "__main__":
    main()
