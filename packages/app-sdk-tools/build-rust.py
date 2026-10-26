#!/usr/bin/env python3
"""Build and admit a Rust App backend; run inside the host's Cargo build slot."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from licenses import rust_notices, select_licenses, toolchain_notices

HERE = Path(__file__).resolve().parent


def main():
    if len(sys.argv) != 3:
        raise RuntimeError('usage: build-rust.py <Cargo.toml> <output.wasm>')
    manifest = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    checker = os.environ.get('CHARIOX_APP_COMPONENT_CHECK')
    if not checker:
        raise RuntimeError('set CHARIOX_APP_COMPONENT_CHECK to the component admission tool')
    pin = json.loads((HERE / 'toolchain.json').read_text())
    actual = subprocess.check_output(['rustc', '--version'], text=True).split()[1]
    if actual != pin['rust']:
        raise RuntimeError(f"use Rust {pin['rust']}, got {actual}")
    # Cargo's path-dependent root crate disambiguator changes Wasm function/type
    # ordering even with remapped panic paths. This root is a standalone cdylib,
    # so a fixed metadata salt is safe; dependency crate salts remain untouched.
    command = ['cargo', 'rustc', '--locked', '--release', '--lib', '--target', pin['target'],
               '--manifest-path', str(manifest), '--message-format=json-render-diagnostics', '--',
               '-Cmetadata=chariox-app-component-v1', f'--remap-path-prefix={manifest.parent}=/app-source']
    process = subprocess.run(command, stdout=subprocess.PIPE, text=True)
    components = []
    for line in process.stdout.splitlines():
        message = json.loads(line)
        if message.get('reason') == 'compiler-message':
            print(message['message'].get('rendered', ''), file=sys.stderr)
        if message.get('reason') == 'compiler-artifact' and message.get('manifest_path') == str(manifest):
            components.extend(file for file in message['filenames'] if file.endswith('.wasm'))
    if process.returncode:
        raise RuntimeError('Rust component build failed; see compiler diagnostics')
    if len(components) != 1:
        raise RuntimeError('expected exactly one root cdylib Wasm artifact')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1', '--manifest-path', str(manifest)], text=True))
    notices = {}
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        selected = select_licenses(package['license'])
        if selected is None:
            raise RuntimeError(f"Rust dependency licence needs review: {package['name']} {package['license']}")
        root = Path(package['manifest_path']).parent
        texts = [p for p in root.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))]
        if not texts and package['source'] is None:
            workspace = Path(metadata['workspace_root'])
            if root.is_relative_to(workspace):
                texts = [p for p in workspace.glob('LICENSE*') if p.is_file()]
        if not texts:
            raise RuntimeError(f"add licence text for Rust dependency {package['name']} {package['version']}")
        for text in sorted(texts):
            notices[f"{package['name']}-{package['version']}/{text.name}"] = text.read_text()
    notices.update(rust_notices())
    lock = manifest.parent / 'Cargo.lock'
    notices['BUILD.json'] = json.dumps({'rust': pin['rust'], 'sdkVersion': '0.1.0', 'lockSha256': hashlib.sha256(lock.read_bytes()).hexdigest()})

    def leb(value):
        encoded = bytearray()
        while True:
            byte = value & 127
            value >>= 7
            encoded.append(byte | (128 if value else 0))
            if not value:
                return encoded

    with tempfile.TemporaryDirectory(prefix='app-rust-', dir=os.environ['TMPDIR']) as temporary:
        sysroot_notices = Path(temporary) / 'sysroot-notices'
        toolchain_notices(sysroot_notices)
        for text in sorted(sysroot_notices.rglob('*')):
            if text.is_file():
                notices['wasi-sdk33/' + text.relative_to(sysroot_notices).as_posix()] = text.read_text()
        admitted = Path(temporary) / 'app.wasm'
        subprocess.run([checker, components[0], str(admitted)], check=True)
        name = b'chariox:licenses'
        payload = leb(len(name)) + name + json.dumps(notices, ensure_ascii=False, sort_keys=True).encode()
        admitted.write_bytes(admitted.read_bytes() + b'\0' + leb(len(payload)) + payload)
        subprocess.run([checker, str(admitted)], check=True)
        pending = output.with_name(output.name + f'.pending-{os.getpid()}')
        try:
            pending.write_bytes(admitted.read_bytes())
            pending.replace(output)
        finally:
            pending.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
