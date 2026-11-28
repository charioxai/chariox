#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: credential-free, hash-bound real TUI runtime bundle."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def build(source, kernel, output, bun, relay=None, codex_vendor=None):
    source = source.resolve()
    if subprocess.check_output(['git','-C',str(source),'status','--porcelain'],text=True).strip():
        raise ValueError('MP-11: source must be committed before runtime provenance is frozen')
    if output.exists():
        raise ValueError('MP-11: runtime output must be new')
    output.mkdir(parents=True, mode=0o700)
    (output / 'bin').mkdir()
    shutil.copy2(kernel, output / 'bin/chariox-kernel')
    shutil.copy2(bun, output / 'bin/bun')
    if relay:
        shutil.copy2(relay, output / 'bin/chariox-relay')
    if codex_vendor:
        # Official credential-free provider distribution, with its native resource layout.
        shutil.copytree(codex_vendor, output / 'providers/codex')
        (output / 'bin/codex').symlink_to('../providers/codex/bin/codex')
        (output / 'bin/codex-code-mode-host').symlink_to('../providers/codex/bin/codex-code-mode-host')
    # Only build assets/dependencies; no provider home, state, config or credentials.
    for name in ['apps/cli/dist', 'packages/kernel-client/dist', 'packages/tool-display/dist']:
        shutil.copytree(source / name, output / name)
    for name in ['node_modules', 'apps/cli/node_modules', 'packages/kernel-client/node_modules']:
        shutil.copytree(source / name, output / name, symlinks=True)
    for name in ['package.json','apps/cli/package.json','packages/kernel-client/package.json','packages/tool-display/package.json']:
        shutil.copy2(source / name, output / name)
    # pnpm workspace dependency is bound to this bundle's built client.
    p = output / 'apps/cli/node_modules/@chariox/kernel-client'
    p.unlink()
    p.symlink_to('../../../../packages/kernel-client')
    p = output / 'apps/cli/node_modules/@chariox/tool-display'
    p.unlink()
    p.symlink_to('../../../../packages/tool-display')
    for p in output.rglob('*'):
        if p.is_symlink() and not p.resolve().is_relative_to(output.resolve()):
            raise ValueError('MP-11: external dependency symlink')
    digest = lambda p: hashlib.file_digest(p.open('rb'), 'sha256').hexdigest()
    manifest = {'source_commit': subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip(),
                'kernel_sha256': digest(output/'bin/chariox-kernel'),
                'files': {str(p.relative_to(output)):digest(p) for p in output.rglob('*') if p.is_file()}}
    (output/'eval-runtime.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return manifest

if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for k in ['source','kernel','output','bun']: p.add_argument('--'+k,type=Path,required=True)
    p.add_argument('--relay',type=Path)
    p.add_argument('--codex-vendor',type=Path)
    a=p.parse_args(); m=build(a.source,a.kernel,a.output,a.bun,a.relay,a.codex_vendor)
    print(json.dumps({k:m[k] for k in ['source_commit','kernel_sha256']}))
