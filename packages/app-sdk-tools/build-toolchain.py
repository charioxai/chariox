#!/usr/bin/env python3
"""Build the one pinned QuickJS compiler; downloads are verified before use."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import urllib.request
import sys

sys.dont_write_bytecode = True
from licenses import notice_digest

HERE = Path(__file__).resolve().parent


def sha256(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def extract(pin, destination):
    archive = destination / 'download.tar.gz'
    with urllib.request.urlopen(pin['url'], timeout=120) as response, archive.open('wb') as output:
        size = 0
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            if size > 1024 * 1024 * 1024:
                raise RuntimeError('toolchain download exceeds 1 GiB')
            output.write(chunk)
    if sha256(archive) != pin['sha256']:
        raise RuntimeError(f"checksum mismatch: {pin['url']}")
    with tarfile.open(archive) as source:
        source.extractall(destination, filter='data')
    archive.unlink()
    roots = list(destination.iterdir())
    if len(roots) != 1 or not roots[0].is_dir():
        raise RuntimeError('unexpected toolchain archive layout')
    return roots[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--cargo-runner', help='build-slot wrapper executable, if required by this host')
    args = parser.parse_args()
    pin = json.loads((HERE / 'toolchain.json').read_text())
    machine = 'aarch64' if platform.machine() in ('arm64', 'aarch64') else platform.machine()
    host = platform.system().lower() + '-' + machine
    assets = pin['platforms'].get(host)
    if assets is None:
        raise RuntimeError(f'no checksummed compiler assets for {host}')
    rust = subprocess.check_output(['rustc', '--version'], text=True).split()[1]
    if rust != pin['rust']:
        raise RuntimeError(f"use Rust {pin['rust']}, got {rust}")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise RuntimeError('output must be empty; keep installed toolchains immutable')
    scratch_root = os.environ.get('TMPDIR')
    if not scratch_root:
        raise RuntimeError('set TMPDIR outside the repository')
    with tempfile.TemporaryDirectory(prefix='app-toolchain-', dir=scratch_root) as temporary:
        scratch = Path(temporary)
        inputs = {}
        for name in ['quickjs', 'wasiSdk', 'binaryen']:
            directory = scratch / name
            directory.mkdir()
            inputs[name] = extract(pin['quickjs'] if name == 'quickjs' else assets[name], directory)
        sdk = inputs['wasiSdk']
        wasm_opt = inputs['binaryen'] / 'bin/wasm-opt'
        if not (sdk / 'bin/clang').is_file() or not wasm_opt.is_file():
            raise RuntimeError('compiler prerequisites missing; refusing upstream fallback downloads')
        if (inputs['quickjs'] / 'crates/core/prebuilt').exists():
            raise RuntimeError('unexpected prebuilt runtime; source build required')
        patch = HERE / 'deterministic-wizer.patch'
        if sha256(patch) != pin['quickjs']['snapshotPatchSha256']:
            raise RuntimeError('snapshot patch checksum mismatch')
        subprocess.run(['git', 'apply', str(patch)], cwd=inputs['quickjs'], check=True)
        cancel_patch = HERE / 'task-cancel-result.patch'
        if sha256(cancel_patch) != pin['quickjs']['cancelPatchSha256']:
            raise RuntimeError('task cancellation patch checksum mismatch')
        subprocess.run(['git', 'apply', str(cancel_patch)], cwd=inputs['quickjs'], check=True)
        env = {**os.environ, 'WASI_SDK_PATH': str(sdk), 'WASM_OPT': str(wasm_opt), 'CARGO_BUILD_JOBS': '4'}
        # Copy while holding the build slot: another lane may reuse its target.
        command = ['sh', '-c', 'cargo build --locked --release -p componentize-qjs-cli && cp "$CARGO_TARGET_DIR/release/componentize-qjs" "$1"', 'app-toolchain', str(output / 'componentize-qjs')]
        if args.cargo_runner:
            command.insert(0, args.cargo_runner)
        elif not env.get('CHARIOX_BUILD_SLOT'):
            env['CARGO_TARGET_DIR'] = str(scratch / 'target')
        subprocess.run(command, cwd=inputs['quickjs'], env=env, check=True)
        # License inventory/texts are generated from this exact Cargo.lock.
        inventory = ['cargo', 'metadata', '--locked', '--format-version=1']
        if args.cargo_runner:
            inventory.insert(0, args.cargo_runner)
        metadata = subprocess.check_output(inventory, cwd=inputs['quickjs'], env=env)
        tree = ['cargo', 'tree', '--locked', '-p', 'componentize-qjs-runtime', '--target', pin['target'], '-e', 'normal,no-proc-macro', '--prefix', 'none']
        if args.cargo_runner:
            tree.insert(0, args.cargo_runner)
        guest = output / 'guest-dependencies.txt'
        guest.write_bytes(subprocess.check_output(tree, cwd=inputs['quickjs'], env=env))
        subprocess.run(['python3', str(HERE / 'licenses.py'), str(output / 'notices'), str(guest)], input=metadata, check=True)
        (output / 'toolchain-build.json').write_text(json.dumps({
            'revision': pin['quickjs']['revision'], 'rust': rust,
            'snapshotPatchSha256': sha256(patch),
            'cancelPatchSha256': sha256(cancel_patch),
            'compilerSha256': sha256(output / 'componentize-qjs'),
            'sourceLockSha256': sha256(inputs['quickjs'] / 'Cargo.lock'),
            'licenseInventorySha256': sha256(output / 'notices/guest/inventory.json'),
            'guestNoticesSha256': notice_digest(output / 'notices/guest'),
            'inputs': pin,
        }, indent=2) + '\n')


if __name__ == '__main__':
    main()
