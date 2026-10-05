#!/usr/bin/env python3
"""MP-03/MP-08/MP-10/MP-11: isolated operator fixture; never a runtime adapter.

The VM operator installs this public file root-owned and grants only its three
commands to the enrolled fixture user. No archive member or payload is output.
"""
import hashlib
import json
import os
import pwd
from pathlib import Path
import stat
import subprocess
import sys


def pinned_open(path, root, flags=os.O_RDONLY, owner=0):
    path = Path(path)
    root = Path(root)
    if not path.is_absolute() or '..' in path.parts or root not in path.parents:
        raise ValueError('fixture path is outside its enrolled artifact root')
    descriptors = []
    try:
        fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        descriptors.append(fd)
        for part in path.parts[1:-1]:
            fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            descriptors.append(fd)
            metadata = os.fstat(fd)
            if metadata.st_uid not in (0, owner) or metadata.st_mode & 0o022:
                raise ValueError('fixture ancestry is not root-controlled')
        result = os.open(path.name, flags | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        metadata = os.fstat(result)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != owner or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != 0o600:
            os.close(result)
            raise ValueError('fixture file is not a private root-owned regular file')
        return result
    finally:
        for fd in reversed(descriptors): os.close(fd)


def apply(operation, state, root, tar=subprocess.run, manifest_root=None, manifest_owner=0):
    if operation not in ('verify', 'corrupt', 'quarantine'):
        raise ValueError('unknown fixture operation')
    if Path(state['manifest_path']).name != 'manifest.json':
        raise ValueError('fixture requires the kernel manifest')
    fd = pinned_open(state['manifest_path'], manifest_root or root, owner=manifest_owner)
    with os.fdopen(fd, 'rb') as stream:
        if os.fstat(stream.fileno()).st_size >= 1024 * 1024:
            raise ValueError('oversized fixture manifest')
        manifest = json.load(stream)
    for key in ('id', 'source_slice_id', 'home_archive_path', 'image_ref', 'size_bytes'):
        if state.get(key) != manifest.get(key): raise ValueError('kernel/manifest fixture mismatch')
    if operation != 'verify' and (state.get('name') != 'corrupt-candidate' or manifest.get('name') != 'corrupt-candidate'):
        raise ValueError('only the disposable corrupt-candidate backup may be altered')
    archive = Path(state['home_archive_path'])
    fd = pinned_open(archive.parent / 'metadata.json', root)
    with os.fdopen(fd, 'rb') as stream:
        if os.fstat(stream.fileno()).st_size > 65536: raise ValueError('oversized private metadata')
        metadata = json.load(stream)
    if metadata.get('schemaVersion') != 1 or metadata.get('id') != state['id'] or metadata.get('sizeBytes') != state['size_bytes']:
        raise ValueError('private capture metadata mismatch')
    if operation != 'verify' and metadata.get('scope') != 'backup':
        raise ValueError('only backup captures may be damaged')
    if operation == 'quarantine':
        quarantines = list(archive.parent.glob(archive.name + '.corrupt-*'))
        for candidate in quarantines:
            fd = pinned_open(candidate, root)
            os.close(fd)
        return {'archivePresent': archive.exists(), 'quarantineCount': len(quarantines)}
    fd = pinned_open(archive, root, os.O_RDWR if operation == 'corrupt' else os.O_RDONLY)
    with os.fdopen(fd, 'r+b' if operation == 'corrupt' else 'rb') as stream:
        before = os.fstat(stream.fileno())
        if before.st_size <= 0 or before.st_size != state['size_bytes']:
            raise ValueError('fixture archive size mismatch')
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if digest != metadata.get('sha256'): raise ValueError('private capture digest mismatch')
        if state.get('home_archive_sha256') and digest != state['home_archive_sha256']:
            raise ValueError('fixture archive digest mismatch')
        if operation == 'corrupt':
            stream.seek(0)
            stream.write(b'deliberately corrupted backup archive')
            stream.truncate()
            stream.flush()
            os.fsync(stream.fileno())
            return {'corrupted': True}
        tar(['/usr/bin/tar', '--zstd', '-tf', f'/proc/self/fd/{stream.fileno()}'],
            pass_fds=(stream.fileno(),), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            check=True, timeout=600, env={'PATH': '/usr/bin:/bin', 'HOME': '/nonexistent'})
        after = archive.stat(follow_symlinks=False)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise ValueError('fixture archive changed during verification')
        return {'sizeBytes': before.st_size, 'sha256': digest, 'private': True, 'readableArchive': True,
                'verification': 'isolated-operator-fixture'}


if __name__ == '__main__':
    try:
        uid = int(os.environ['SUDO_UID'])
        if os.geteuid() != 0 or uid <= 0 or len(sys.argv) != 2:
            raise ValueError('fixture requires the isolated operator sudo grant')
        raw = sys.stdin.buffer.read(65537)
        if len(raw) > 65536: raise ValueError('oversized fixture request')
        root = Path(f'/var/lib/chariox/slice-local-dev/u-{uid}/private/layout/share/.broker-private/artifacts')
        manifest_root = Path(pwd.getpwuid(uid).pw_dir) / '.chariox/dev'
        print(json.dumps(apply(sys.argv[1], json.loads(raw), root,
                               manifest_root=manifest_root, manifest_owner=uid)))
    except Exception:
        # Errors from archive tools and manifests may contain private data.
        print('MP-03/MP-10: isolated storage fixture refused', file=sys.stderr)
        sys.exit(1)
