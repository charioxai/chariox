#!/usr/bin/env python3
"""Explicit fresh Linux rootful slice DEV enrollment. Never migrates private assets.
Run after building/attesting the worker image; root publishes public source/image
pins. The trusted helper is built here from that immutable public source.
"""
import argparse, grp, hashlib, json, os, pathlib, pwd, re, shutil, stat, subprocess, tempfile

ENV = {'PATH': '/usr/bin:/bin', 'HOME': '/nonexistent', 'DOCKER_HOST': 'unix:///run/docker.sock', 'DOCKER_CONFIG': '/nonexistent'}
def refuse(message): raise SystemExit('Local Docker DEV enrollment refused: ' + message)
def command(args, environment=ENV):
    return subprocess.run(['/usr/bin/docker', *args], env=environment, check=True, capture_output=True, text=True, timeout=1200).stdout

def directory(path, mode):
    try: path.mkdir(mode=mode)
    except FileExistsError: pass
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or stat.S_IMODE(metadata.st_mode) != mode or path.resolve() != path:
        refuse('incompatible existing public/control directory')

def publish(path, payload, mode):
    if path.exists() or path.is_symlink():
        m = path.lstat()
        if not stat.S_ISREG(m.st_mode) or m.st_uid != 0 or m.st_nlink != 1 or stat.S_IMODE(m.st_mode) != mode or path.read_bytes() != payload:
            refuse('existing enrollment differs; private state was not changed')
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try: os.write(fd, payload); os.fsync(fd)
    finally: os.close(fd)

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--allow-provider-sandbox-compatibility', action='store_true', required=True,
    help='Grant local DEV workers relaxed seccomp, unmasked system paths and the selected AppArmor profile for provider namespaces')
p.add_argument('--source', required=True, type=pathlib.Path)
p.add_argument('--user', required=True)
p.add_argument('--worker-image', required=True)
p.add_argument('--worker-kernel-sha256', required=True)
p.add_argument('--worker-runtime-revision', required=True, help='Reviewed runtime-source-revision label of the immutable worker image')
p.add_argument('--node-runtime', required=True, type=pathlib.Path)
p.add_argument('--node-runtime-sha256', required=True)
p.add_argument('--buildx-runtime', required=True, type=pathlib.Path)
p.add_argument('--buildx-runtime-sha256', required=True)
p.add_argument('--docker-cli-sha256', required=True, help='Reviewed SHA-256 of the public host Docker CLI copied into the helper')
a = p.parse_args()
if os.geteuid() != 0: refuse('root installation required')
user = pwd.getpwnam(a.user)
if not re.fullmatch('sha256:[a-f0-9]{64}', a.worker_image) or not re.fullmatch('[a-f0-9]{64}', a.worker_kernel_sha256): refuse('immutable image/kernel pins required')
socket = pathlib.Path('/run/docker.sock').lstat()
if not stat.S_ISSOCK(socket.st_mode) or socket.st_uid != 0 or socket.st_mode & 0o007: refuse('canonical rootful socket required')
# Enrollment grants no Docker socket permission. The ordinary launch must
# succeed using its real current groups; do not mutate account memberships.
engine = json.loads(command(['info', '--format', '{{json .}}']))
if engine.get('OSType') != 'linux' or any(re.search('rootless|userns', x) for x in engine.get('SecurityOptions', [])): refuse('only unmapped Linux rootful engine supported')
worker = json.loads(command(['image', 'inspect', a.worker_image]))[0]
if worker['Id'] != a.worker_image or worker['Config']['User'] != 'slice': refuse('worker image identity/user mismatch')
if not re.fullmatch('(sha256:)?[a-f0-9]{64}', a.worker_runtime_revision) or worker['Config'].get('Labels', {}).get('io.chariox.runtime-source-revision') != a.worker_runtime_revision: refuse('worker runtime revision pin mismatch')
proof = command(['run', '--rm', '--read-only', '--network', 'none', '--cap-drop', 'ALL', '--memory', '64m', '--pids-limit', '16', '--user', '0:0', '--entrypoint', '/usr/bin/sha256sum', a.worker_image, '/opt/chariox-slice/bin/chariox-kernel']).split()[0]
if proof != a.worker_kernel_sha256: refuse('actual worker runtime hash mismatch')
source = a.source.resolve()
# Public CLI bytes only. The exact pin enters the immutable source manifest;
# no Docker configuration or credential directory is copied.
cli = pathlib.Path('/usr/bin/docker')
metadata = cli.lstat()
if not re.fullmatch('[a-f0-9]{64}', a.docker_cli_sha256): refuse('public Docker CLI pin required')
if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_nlink != 1 or metadata.st_mode & 0o022 or cli.resolve() != cli:
    refuse('public Docker CLI must be a root-controlled regular file')
cli_bytes = cli.read_bytes()
if len(cli_bytes) > 64 * 1024 * 1024 or hashlib.sha256(cli_bytes).hexdigest() != a.docker_cli_sha256:
    refuse('public Docker CLI pin mismatch or oversized binary')
node = a.node_runtime
if not node.is_absolute() or not re.fullmatch('[a-f0-9]{64}', a.node_runtime_sha256): refuse('absolute public Node runtime and reviewed pin required')
for entry in [node, *node.parents]:
    m = entry.lstat()
    if stat.S_ISLNK(m.st_mode) or m.st_uid != 0 or m.st_mode & 0o022: refuse('Node runtime ancestry must be root-controlled')
    if entry == node and (not stat.S_ISREG(m.st_mode) or m.st_nlink != 1): refuse('Node runtime must be a single regular file')
    if entry != node and not stat.S_ISDIR(m.st_mode): refuse('Node runtime ancestor is not a directory')
node_bytes = node.read_bytes()
if len(node_bytes) > 128 * 1024 * 1024 or hashlib.sha256(node_bytes).hexdigest() != a.node_runtime_sha256: refuse('public Node runtime pin mismatch')
# Execute only root-controlled, explicitly pinned public bytes with no profiles.
loaded = subprocess.run([str(node), '--version'], env={'PATH':'/usr/bin:/bin','HOME':'/nonexistent'}, capture_output=True, text=True, timeout=10)
if loaded.returncode or not re.fullmatch(r'v22\.[0-9]+\.[0-9]+\n?', loaded.stdout): refuse('public Node loader/version preflight failed')
plugin = a.buildx_runtime
if not plugin.is_absolute() or not re.fullmatch('[a-f0-9]{64}', a.buildx_runtime_sha256): refuse('absolute public Buildx executable and reviewed pin required')
for entry in [plugin, *plugin.parents]:
    m = entry.lstat()
    if stat.S_ISLNK(m.st_mode) or m.st_uid != 0 or m.st_mode & 0o022: refuse('Buildx ancestry must be root-controlled')
    if entry == plugin and (not stat.S_ISREG(m.st_mode) or m.st_nlink != 1): refuse('Buildx must be a single regular file')
    if entry != plugin and not stat.S_ISDIR(m.st_mode): refuse('Buildx ancestor is not a directory')
plugin_bytes = plugin.read_bytes()
if len(plugin_bytes) > 128 * 1024 * 1024 or hashlib.sha256(plugin_bytes).hexdigest() != a.buildx_runtime_sha256: refuse('public Buildx pin mismatch')
files = [('.local-public-tools/docker', cli_bytes), ('.local-public-tools/node', node_bytes)]
for name in subprocess.check_output(['git', '-C', str(source), 'ls-files', 'apps/kernel/slice-linux-docker', 'apps/kernel/src/transport/relay_peer.rs', 'apps/browser-session-import'], text=True).splitlines():
    relative = pathlib.PurePosixPath(name)
    if 'prebuilt' in relative.parts: continue
    original = source / name
    if original.is_symlink() or not original.is_file() or original.resolve() != original or source not in original.parents: refuse('non-regular or redirected source input')
    if original.stat().st_size > 16 * 1024 * 1024: refuse('oversized source input')
    files.append((str(relative), original.read_bytes()))
# Include staged owned source changes only after their normal tracked publication.
required = {f'apps/kernel/slice-linux-docker/{name}' for name in ['protected-local-docker-authority.mjs', 'protected-authority.mjs', 'local-docker-broker-launch.mjs', 'docker/LocalBroker.Dockerfile']}
if not required <= {name for name, _ in files}: refuse('local implementation must be tracked in the selected source')
manifest = json.dumps({'version': 1, 'files': [{'path': name, 'sha256': hashlib.sha256(data).hexdigest()} for name, data in sorted(files)]}, separators=(',', ':')).encode()
digest = hashlib.sha256(manifest).hexdigest()
for parent in ['/usr/lib/chariox', '/usr/lib/chariox/slice-local-dev', '/etc/chariox', '/etc/chariox/slice-local-dev', '/var/lib/chariox', '/var/lib/chariox/slice-local-dev']:
    directory(pathlib.Path(parent), 0o755)
root = pathlib.Path('/usr/lib/chariox/slice-local-dev') / digest
directory(root, 0o755)
for name, data in files:
    target = root / name
    for parent in reversed(list(target.parent.parents)):
        if parent != root and root not in parent.parents: continue
        directory(parent, 0o755)
    directory(target.parent, 0o755)
    publish(target, data, 0o555 if name.endswith('.sh') or name == '.local-public-tools/node' else 0o444)
publish(root / 'source-manifest.json', manifest, 0o444)
# Unique tag and iid receipt. No profile, credential or private state enters context.
with tempfile.TemporaryDirectory(prefix='chariox-local-broker-build-', dir='/run') as scratch:
    iid = pathlib.Path(scratch) / 'image.id'
    config = pathlib.Path(scratch) / 'docker-config'
    plugins = config / 'cli-plugins'; plugins.mkdir(parents=True, mode=0o700)
    verified_plugin = plugins / 'docker-buildx'
    publish(verified_plugin, plugin_bytes, 0o555)
    build_environment = {**ENV, 'DOCKER_CONFIG': str(config)}
    command(['buildx', 'version'], build_environment)
    builder = f'chariox-local-helper-{os.getpid()}-{digest[:12]}'
    created = False
    try:
        command(['buildx', 'create', '--name', builder, '--driver', 'docker-container'], build_environment)
        created = True
        command(['buildx', 'build', '--builder', builder, '--load', '--network', 'default', '--iidfile', str(iid), '--build-arg', f'CHARIOX_LOCAL_SOURCE_DIGEST=sha256:{digest}', '--build-arg', f'CHARIOX_LOCAL_DOCKER_SHA256={a.docker_cli_sha256}', '-f', str(root / 'apps/kernel/slice-linux-docker/docker/LocalBroker.Dockerfile'), '-t', f'chariox-local-broker-dev:{digest}', str(root)], build_environment)
    finally:
        if created: command(['buildx', 'rm', builder], build_environment)
    helper = iid.read_text().strip()
if not re.fullmatch('sha256:[a-f0-9]{64}', helper): refuse('helper build identity unavailable')
# Test loader/dependencies in the actual pinned helper, then negotiate with the
# exact enrolled engine. These probes have no private mounts or profiles.
command(['run', '--rm', '--read-only', '--network', 'none', '--cap-drop', 'ALL', '--memory', '64m', '--pids-limit', '16', '--entrypoint', '/bin/sh', helper, '-ec', '/usr/bin/docker --version; /usr/bin/python3 --version; /usr/bin/zstd --version; /usr/bin/tar --version'])
helper_engine = command(['run', '--rm', '--read-only', '--network', 'none', '--cap-drop', 'ALL', '--memory', '64m', '--pids-limit', '16', '--mount', 'type=bind,src=/run/docker.sock,dst=/run/docker.sock', '--env', 'DOCKER_HOST=unix:///run/docker.sock', '--env', 'DOCKER_CONFIG=/nonexistent', '--entrypoint', '/usr/bin/docker', helper, 'info', '--format', '{{.ID}}']).strip()
if helper_engine != engine['ID']: refuse('helper CLI does not reach the enrolled engine')
owner_root = pathlib.Path(f'/var/lib/chariox/slice-local-dev/u-{user.pw_uid}')
directory(owner_root, 0o755)
directory(owner_root / 'private', 0o700)
layout = owner_root / 'private/layout'
directory(layout, 0o711)
for child in ['homes', 'receipts', 'images', 'backups', 'share', 'handles']:
    directory(layout / child, 0o711 if child == 'homes' else 0o700)
directory(layout / 'share/.broker-private', 0o700)
directory(layout / 'share/.broker-private/output', 0o700)
directory(layout / 'share/.broker-private/artifacts', 0o700)
record = {'version': 1, 'topology': 'linux-local-rootful-dev', 'ownerUid': user.pw_uid, 'ownerGid': user.pw_gid, 'engineId': engine['ID'], 'socket': {'path': '/run/docker.sock', 'dev': socket.st_dev, 'ino': socket.st_ino, 'uid': socket.st_uid, 'gid': socket.st_gid, 'mode': stat.S_IMODE(socket.st_mode)}, 'helperImageId': helper, 'workerImageId': a.worker_image, 'workerKernelHash': a.worker_kernel_sha256, 'workerRuntimeRevision': a.worker_runtime_revision, 'sourceDigest': f'sha256:{digest}', 'sourceRoot': str(root), 'controlRoot': str(layout)}
publish(pathlib.Path(f'/etc/chariox/slice-local-dev/{user.pw_uid}.json'), json.dumps(record, separators=(',', ':')).encode(), 0o644)
launcher = f'#!/bin/sh\nexec {root}/.local-public-tools/node {root}/apps/kernel/slice-linux-docker/local-docker-broker-launch.mjs\n'.encode()
publish(pathlib.Path(f'/usr/libexec/chariox-local-docker-broker-{user.pw_uid}'), launcher, 0o555)
print(json.dumps({'topology': record['topology'], 'ownerUid': user.pw_uid, 'sourceDigest': record['sourceDigest'], 'helperImageId': helper, 'workerImageId': a.worker_image, 'workerKernelHash': proof, 'workerRuntimeRevision': a.worker_runtime_revision}))
