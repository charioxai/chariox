#!/usr/bin/env python3
"""Check the pinned Cargo inventory and preserve its dependency notices."""
import hashlib
import io
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile

ALLOWED = {
    'MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-2-Clause',
    'BSD-3-Clause', 'ISC', 'Zlib', 'CC0-1.0', 'Unlicense', '0BSD',
    'Unicode-3.0', 'Unicode-DFS-2016', 'MPL-2.0', 'BSL-1.0',
    'NCSA', 'BlueOak-1.0.0', 'CDLA-Permissive-2.0', 'bzip2-1.0.6',
}


def rust_notices():
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    docs = sysroot / 'share/doc/rust'
    copyright = docs / 'COPYRIGHT-library.html'
    if not copyright.is_file():
        raise RuntimeError('install rust-docs for the pinned compiler to preserve Rust standard-library notices')
    notices = {'rust/COPYRIGHT-library.html': copyright.read_text()}
    for text in sorted((docs / 'licenses').glob('*.txt')):
        notices[f'rust/licenses/{text.name}'] = text.read_text()
    return notices


def notice_digest(directory):
    files = {path.relative_to(directory).as_posix(): path.read_bytes().decode('utf8') for path in sorted(directory.rglob('*')) if path.is_file()}
    return hashlib.sha256(json.dumps(files, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def npm_notices(packages, output):
    inventory = []
    for package in packages:
        selected = select_licenses(package['license'])
        if selected is None:
            raise RuntimeError(f"npm licence review required: {package['name']} {package['version']}: {package['license']}")
        root = Path(package['directory'])
        texts = [path for path in root.iterdir() if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))]
        if not texts:
            raise RuntimeError(f"npm dependency lacks licence text: {package['name']}")
        destination = output / (package['name'].replace('/', '--') + '-' + package['version'])
        destination.mkdir(parents=True)
        for text in texts:
            shutil.copyfile(text, destination / text.name)
        if 'MPL-2.0' in selected:
            for file in package['inputs']:
                source = Path(file)
                target = destination / 'sources' / source.relative_to(root)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
            shutil.copyfile(root / 'package.json', destination / 'sources/package.json')
        inventory.append({key: package[key] for key in ('name', 'version', 'license')} | {'selected': sorted(selected)})
    output.mkdir(parents=True, exist_ok=True)
    (output / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')


def select_licenses(expression):
    # Cargo's legacy slash spelling means OR.
    tokens = re.findall(r'\(|\)|[^\s()]+', (expression or '').replace('/', ' OR '))
    position = 0

    def term():
        nonlocal position
        if position >= len(tokens):
            raise ValueError('incomplete SPDX expression')
        token = tokens[position]
        position += 1
        if token == '(':
            value = expression_or()
            if position >= len(tokens) or tokens[position] != ')':
                raise ValueError('unclosed SPDX expression')
            position += 1
            return value
        if position < len(tokens) and tokens[position] == 'WITH':
            if position + 1 >= len(tokens):
                raise ValueError('missing SPDX exception')
            token += ' WITH ' + tokens[position + 1]
            position += 2
        return {token} if token in ALLOWED else None

    def expression_and():
        nonlocal position
        value = term()
        while position < len(tokens) and tokens[position] == 'AND':
            position += 1
            right = term()
            value = value | right if value is not None and right is not None else None
        return value

    def expression_or():
        nonlocal position
        value = expression_and()
        while position < len(tokens) and tokens[position] == 'OR':
            position += 1
            right = expression_and()
            value = value if value is not None else right
        return value

    try:
        value = expression_or()
        return value if position == len(tokens) else None
    except ValueError:
        return None


def permitted(expression):
    return select_licenses(expression) is not None


def main():
    output = Path(sys.argv[1])
    metadata = json.load(sys.stdin)
    guest = {tuple(line.replace('(*)', '').split()[:2]) for line in Path(sys.argv[2]).read_text().splitlines() if len(line.split()) >= 2}
    guest = {(name, version.removeprefix('v')) for name, version in guest}
    guest.add(('wasi-preview1-component-adapter-provider', '49.0.4'))
    sources = json.loads(Path(__file__).with_name('notice-sources.json').read_text())
    upstream_texts = {}
    for source in sources:
        destination = output / 'guest' / 'toolchain' / source['name']
        destination.parent.mkdir(parents=True, exist_ok=True)
        import urllib.request
        with urllib.request.urlopen(source['url'], timeout=60) as response:
            data = response.read(50 * 1024 * 1024)
        if hashlib.sha256(data).hexdigest() != source['sha256']:
            raise RuntimeError(f"notice checksum mismatch: {source['name']}")
        if source.get('archive'):
            notices = []
            with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
                for member in archive:
                    path = Path(member.name)
                    if not member.isfile() or path.suffix.lower() not in ('.c', '.h', '.s'):
                        continue
                    if not any(part in path.parts for part in ('libc-top-half', 'libc-bottom-half', 'dlmalloc', 'emmalloc', 'fts')):
                        continue
                    stream = archive.extractfile(member)
                    text = stream.read().decode('utf8', errors='replace')
                    comments = re.findall(r'/\*.*?\*/|(?://[^\n]*\n)+', text, re.S)
                    comments = [comment for comment in comments if re.search(r'copyright|licen[cs]e|permission|public.domain|all rights|redistribut', comment, re.I)]
                    if any(re.search(r'\b[AL]?GPL\b|GNU (?:General|Lesser)', comment) for comment in comments):
                        raise RuntimeError(f'copyleft source notice needs review: {member.name}')
                    if comments:
                        notices.append(f'\nFile: {member.name}\n' + '\n'.join(comments))
            destination.write_text('\n'.join(notices))
        else:
            destination.write_bytes(data)
        for name in source.get('packages', []):
            upstream_texts.setdefault(name, []).append(destination)

    def local_texts(package):
        root = Path(package['manifest_path']).parent
        return [path for path in root.rglob('*') if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))]

    by_revision = {}
    for package in metadata['packages']:
        vcs = Path(package['manifest_path']).with_name('.cargo_vcs_info.json')
        if vcs.exists():
            revision = json.loads(vcs.read_text()).get('git', {}).get('sha1')
            by_revision.setdefault(revision, []).extend(local_texts(package))
    inventory = []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        license_expression = package['license']
        if package['name'] == 'wit-dylib-ffi' and 'e22db44da4eaf17a0e46ec55b911c7539589a3cc' in (package['source'] or ''):
            license_expression = 'MIT'
        if license_expression is None and package['source'] is None and package['publish'] == []:
            continue  # Unpublished workspace build task, never linked into output.
        if not permitted(license_expression):
            raise RuntimeError(f"licence review required: {package['name']} {package['version']}: {license_expression}")
        root = Path(package['manifest_path']).parent
        shipped = (package['name'], package['version']) in guest
        texts = local_texts(package) if shipped else []
        if package.get('license_file'):
            texts.append(root / package['license_file'])
        # Workspace members inherit the root licence.
        if not texts and package['source'] is None:
            workspace = Path(metadata['workspace_root'])
            if root.is_relative_to(workspace):
                texts = [path for path in workspace.glob('LICENSE*') if path.is_file()]
        if not texts and shipped:
            vcs = root / '.cargo_vcs_info.json'
            if vcs.exists():
                revision = json.loads(vcs.read_text()).get('git', {}).get('sha1')
                texts = by_revision.get(revision, [])
            texts = texts or upstream_texts.get(package['name'], [])
        if not texts and shipped:
            raise RuntimeError(f"missing licence text: {package['name']} {package['version']}")
        if not shipped:
            texts = []
        destination = output / 'guest' / f"{package['name']}-{package['version']}"
        if shipped:
            destination.mkdir(parents=True)
        digests = {}
        for index, path in enumerate(sorted(set(texts))):
            name = f'{index}-{path.name}'
            shutil.copyfile(path, destination / name)
            digests[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        inventory.append({
            'name': package['name'], 'version': package['version'], 'license': license_expression,
            'selected': sorted(select_licenses(license_expression)), 'shipped': shipped,
            'source': package['source'], 'repository': package.get('repository'), 'notices': digests,
            **({'note': 'MIT declared; copyright holder and licence text unavailable upstream. Host compiler only; do not distribute this CLI binary.'} if package['name'] == 'json-escape-simd' else {}),
        })
    output.mkdir(parents=True, exist_ok=True)
    (output / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    (output / 'guest' / 'inventory.json').write_text(json.dumps([p for p in inventory if p['shipped']], indent=2) + '\n')
    for name, text in rust_notices().items():
        destination = output / 'guest' / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)
    print(f'licence inventory: {len(inventory)} packages; notices preserved')


if __name__ == '__main__':
    if sys.argv[1] == '--npm':
        npm_notices(json.load(sys.stdin), Path(sys.argv[2]))
    else:
        main()
