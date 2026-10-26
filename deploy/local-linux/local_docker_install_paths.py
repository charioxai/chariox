"""MP-03/MP-11: root-controlled filesystem publication for local DEV enrollment."""
import os
import pathlib
import stat


def refuse(message):
    raise SystemExit('Local Docker DEV enrollment refused: ' + message)


def root_directory(path):
    path = pathlib.Path(path)
    if not path.is_absolute() or '..' in path.parts:
        refuse('absolute canonical root-controlled directory required')
    for entry in [*reversed(path.parents), path]:
        metadata = entry.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0
                or metadata.st_mode & 0o022):
            refuse('directory ancestry must be root-controlled')


def directory(path, mode):
    # Validate BEFORE mkdir: a symlink or foreign parent must receive no writes.
    root_directory(path.parent)
    try:
        path.mkdir(mode=mode)
    except FileExistsError:
        pass
    root_directory(path)
    if stat.S_IMODE(path.lstat().st_mode) != mode:
        refuse('incompatible existing public/control directory')


def publish(path, payload, mode):
    root_directory(path.parent)
    if path.exists() or path.is_symlink():
        metadata = path.lstat()
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0
                or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != mode
                or path.read_bytes() != payload):
            refuse('existing enrollment differs; private state was not changed')
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, 'wb') as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())
