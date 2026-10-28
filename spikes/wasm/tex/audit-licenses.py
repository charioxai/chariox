#!/usr/bin/env python3
"""MP-08/MP-11: pin upstream licence evidence; never infer from package badges."""
import concurrent.futures, datetime, hashlib, json, pathlib, sys, urllib.request

SOURCES = [
 ('overleaf', 'overleaf/overleaf', ['LICENSE', 'libraries/overleaf-editor-core/LICENSE', 'libraries/overleaf-editor-core/package.json']),
 ('tectonic', 'mgilbir/tectonic', ['LICENSE', 'crates/engine_xdvipdfmx/xdvipdfmx/dvipdfmx.c', 'crates/engine_xdvipdfmx/build.rs', 'crates/engine_xetex/xetex/teckit-Engine.cpp', 'crates/engine_xetex/build.rs']),
 ('tecgonic', 'mgilbir/tecgonic', ['LICENSE', 'Makefile', 'Dockerfile']),
 ('swiftlatex', 'SwiftLaTeX/SwiftLaTeX', ['LICENSE', 'README.md']),
 ('texlive-js', 'manuels/texlive.js', ['LICENSE', 'README.md']),
 ('busytex-upstream', 'busytex/busytex', ['LICENSE', 'README.md']),
 ('busytex-texlyre', 'TeXlyre/texlyre-busytex-build', ['LICENSE', 'README.md']),
 ('codemirror', 'codemirror/dev', ['LICENSE']),
 ('pdfjs', 'mozilla/pdf.js', ['LICENSE']),
 ('harfbuzz', 'harfbuzz/harfbuzz', ['COPYING']),
 ('icu', 'unicode-org/icu', ['LICENSE']),
 ('freetype', 'freetype/freetype', ['LICENSE.TXT', 'docs/FTL.TXT']),
 ('graphite2', 'silnrsi/graphite', ['COPYING']),
 ('libpng', 'pnggroup/libpng', ['LICENSE']),
 ('zlib', 'madler/zlib', ['LICENSE']),
 ('teckit', 'silnrsi/teckit', ['LICENSING.txt']),
 ('typst', 'typst/typst', ['LICENSE']),
 ('wasi-libc', 'WebAssembly/wasi-libc', ['LICENSE']),
 ('llvm', 'llvm/llvm-project', ['libcxx/LICENSE.TXT', 'libcxxabi/LICENSE.TXT']),
 ('latex', 'latex3/latex2e', ['LICENSE', 'base/lppl.txt']),
 ('latinmodern', 'TeX-Live/texlive-source', ['COPYING', 'LICENSE.TL']),
]

def get(url):
    with urllib.request.urlopen(url, timeout=60) as r: return r.read()

def fetch(item, out):
    name, repo, files = item
    meta = json.loads(get(f'https://api.github.com/repos/{repo}'))
    ref = 'wasm' if name == 'tectonic' else meta['default_branch']
    sha = json.loads(get(f'https://api.github.com/repos/{repo}/commits/{ref}'))['sha']
    result = {'part': name, 'repo': repo, 'commit': sha, 'date': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'files': []}
    for file in files:
        url = f'https://raw.githubusercontent.com/{repo}/{sha}/{file}'
        try:
            data = get(url); dest = out/name/file; dest.parent.mkdir(parents=True, exist_ok=True); dest.write_bytes(data)
            result['files'].append({'path': file, 'url': url, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
        except Exception as e:
            result['files'].append({'path': file, 'url': url, 'error': str(e)})
    return result

if __name__ == '__main__':
    out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futures = {pool.submit(fetch, item, out): item[0] for item in SOURCES}
        for future in concurrent.futures.as_completed(futures):
            try: rows.append(future.result())
            except Exception as e: rows.append({'part': futures[future], 'error': str(e)})
    (out/'manifest.json').write_text(json.dumps(sorted(rows, key=lambda r: r['part']), indent=2)+'\n')
    print(json.dumps([{'part': r['part'], 'commit': r.get('commit'), 'ok': sum('sha256' in f for f in r.get('files', [])), 'error': r.get('error')} for r in rows]))
