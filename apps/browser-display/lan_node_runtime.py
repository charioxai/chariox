"""MP-08/MP-10: only the official, self-contained linux-x64 Node runtime."""
import hashlib
import tarfile
from pathlib import Path

VERSION = '22.20.0'
ARCHIVE = f'node-v{VERSION}-linux-x64.tar.xz'
SHA256 = '00bbd05e306ea68b6e13e17360d0e2f680b493ef95f2fea1c4296ff7437530bc'
URL = f'https://nodejs.org/dist/v{VERSION}/{ARCHIVE}'

def install(archive, destination):
    archive, destination = Path(archive), Path(destination)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise ValueError('MP-08/MP-10: official Node archive checksum mismatch')
    with tarfile.open(archive) as package:
        member = package.getmember(f'node-v{VERSION}-linux-x64/bin/node')
        if not member.isfile():
            raise ValueError('MP-08/MP-10: Node binary must be a regular file')
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(package.extractfile(member).read())
        destination.chmod(0o755)
    return {'version': VERSION, 'url': URL, 'archive_sha256': SHA256,
            'binary_sha256': hashlib.sha256(destination.read_bytes()).hexdigest()}
