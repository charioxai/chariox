# A LaTeX App for Chariox

MP-08 / MP-10 / MP-11 · research only · 2026-10-22

**Verdict: works with caveats technically; does not qualify as a first-party
LaTeX App under the accepted licence rule.** Overleaf CE is AGPL. Tectonic's
MIT label does **not** cover all the engine code: its linked PDF converter
contains GPL-2.0-or-later source, and its TECkit code offers CPL/LGPL. That
changes the original recommendation. Keep the permissive rule. Prefer a
Typst example or a wrapped, separately licensed LaTeX engine until there is
an acceptable engine licence route.

An original editor using CodeMirror 6 and PDF.js is feasible. Both UI
libraries pass the rule. This spike builds Tectonic directly for
`wasm32-wasip2` and uses the existing async worker, App WIT and broker under
bubblewrap. It does not implement an App, change the kernel, or resume Apps
implementation. No preview 1 adapter was needed. No MP acceptance item is closed by compiler
measurements.

## Measurements

All samples and conditions are in [results.json](results.json); exact commands
are in [README.md](README.md). Raw logs, PDFs, page renders, licences and
identities stay outside git at `/root/.codex/evidence/texspike/`.

The component is **4.23 MB**, its AOT cache **12.19 MB**; precompiling it took
**0.37 s** (one precompile measurement). AOT-ready means tools registered, before loading TeX data.

Three-run medians on shared b3, seconds (cold includes actual downloads):

| Engine / document | Cold compile | Warm | One-line edit |
|---|---:|---:|---:|
| Component / article | 83.70 | 0.67 | 0.66 |
| Component / 10-page paper | 86.32 | 1.18 | 0.78 |
| Component / beamer | 85.33 | 1.34 | 2.27 |
| Native Tectonic / article | 72.72 | 0.26 | 0.26 |
| Native Tectonic / 10-page paper | 106.85 | 0.62 | 0.62 |
| Native Tectonic / beamer | 120.66 | 0.67 | 1.17 |
| Native Typst / equivalent 10-page paper | 0.31 | 0.31 | 0.31 |

Component article AOT-ready: **7.9–9.4 ms**. Its three first compiles fetched
**16,780,219 bytes each** (440 files including 16 font binaries); warm/edit fetched zero.
Warm article samples were **0.53, 2.54, 0.67 s**: do not infer a P95 or quiet-host
latency guarantee from three samples. The process tree's measured cgroup
peak was **397–399 MiB**, with a real **512 MiB memory.max and zero swap**;
there were no OOM events. The guest linear-memory cap was also 512 MiB.

All **27 component compile cells** passed across nine successful series.
There were **ten cold attempts**: one beamer download received HTTP 200
instead of 206 for `umsb.fd`, was rejected, and failed before compiling.
The failed row/log is retained in `failedComponentAttempts`; one deliberate
fresh-cache retry passed. Thus this is not a claim of 100% download
reliability. A real App needs bounded download recovery and useful progress.

Native Tectonic peak RSS was up to **238 MiB**; native Typst about **40 MiB**.
The component worker's peak RSS was up to **327 MiB**, and all successful
process-tree peaks stayed below **400 MiB**. The 512 MiB worker test does
not include a future editor view/browser or the parent HTTP proxy. The
matrix explicitly used a 30-second CPU-slice setting; an additional warm
paper control also passed at the normal 1-second setting. Every final PDF was
rendered page by page; no blank pages were found. All three paper outputs
retain the bibliography, and pages 1–9 match the native render pixel for
pixel (the edit changes page 10).

The native comparator launches a fresh CLI process each time; the component
keeps its App instance. Native cold caches discover each document's own
files; the component uses the same frozen union subset for every document.
Cold numbers therefore include different download inventories, not just
engine speed. Typst was measured as a **native CLI**, not a new component.
`pdflatex` is not installed on b3; no number was substituted.

The corpus is an article; a **10-page paper** with amsmath, an actual PNG,
an actual TikZ diagram, hyperref and BibTeX; and a seven-page beamer deck (the appended edit adds an eighth page in
both engines).
The Typst comparison uses the same text, math, PNG and bibliography, with a
native vector diagram in place of TikZ. It is not a LaTeX compatibility test.

Numbers are research results on shared b3, not quiet-host performance
acceptance. The unmodified worker cannot open `data`; the red run records
that failure. The spike worker adds only the harness-owned `data` and `tmp`
preopens. A second red run reached format creation, then aborted: the fork
reads `latex.fmt` from the bundle but generates it in the cache. Matching
tecgonic's copy-to-bundle step fixed that seam. Both failures and passing
reruns are retained. Guest TCP, UDP and DNS remain denied and bubblewrap
has no network.

## Licences checked today

Checked **2026-10-22** against upstream licence files and the versions actually
linked. `audit-licenses.py` records exact Git commits, URLs and hashes; raw
texts are in the external evidence directory. "Yes" means the project's
§7b policy allows the licence, **with** its notices and other obligations;
it is not a blanket clearance of an unexamined release. The complete
engine fails the policy even where individual rows pass.

| Part | Upstream licence evidence | First-party §7b? |
|---|---|---|
| Overleaf CE web, CLSI and service code | [AGPL-3.0](https://github.com/overleaf/overleaf/blob/main/LICENSE), repeated in [CLSI](https://github.com/overleaf/overleaf/blob/main/services/clsi/LICENSE) | **No** |
| Overleaf's bundled ShareJS fork | [MIT](https://github.com/overleaf/overleaf/blob/main/services/document-updater/app/js/sharejs/LICENSE) | Yes as a separately assessed library; does not relicense the CE services |
| Overleaf editor-core package metadata | Reports Proprietary; no separate LICENSE found there | No independent clearance from this spike; CE's root licence does not make every third-party or separately marked part permissive |
| Tectonic's new Rust/C code | [MIT, with an explicit inherited-code qualification](https://github.com/tectonic-typesetting/tectonic/blob/master/LICENSE) | Yes for those files; **not clearance of the engine** |
| Tectonic xdvipdfmx and PDF I/O | [GPL-2.0-or-later source](https://github.com/mgilbir/tectonic/blob/d1a46a5d127ea40c6a1afcb45e91bb3d7c1f5e47/crates/engine_xdvipdfmx/xdvipdfmx/dvipdfmx.c), [actually compiled](https://github.com/mgilbir/tectonic/blob/d1a46a5d127ea40c6a1afcb45e91bb3d7c1f5e47/crates/engine_xdvipdfmx/build.rs) | **No**. A WASM build does not remove the source's terms |
| TECkit, bundled inside XeTeX | [CPL-0.5-or-later OR LGPL-2.1-or-later](https://github.com/silnrsi/teckit/blob/master/license/LICENSING.txt); [included source](https://github.com/mgilbir/tectonic/blob/d1a46a5d127ea40c6a1afcb45e91bb3d7c1f5e47/crates/engine_xetex/xetex/teckit-Engine.cpp) | **No under the present whitelist**; MPL in some other TECkit files does not relicense this source |
| HarfBuzz, submodule 894a1f72 | [Old MIT](https://github.com/harfbuzz/harfbuzz/blob/894a1f72ee93a1fd8dc1d9218cb3fd8f048be29a/COPYING) | Yes, preserve notices |
| ICU 74.2, common + stubdata | [Unicode-3.0 and third-party notices](https://github.com/unicode-org/icu/blob/release-74-2/LICENSE) | Permissive; yes if §7b's MIT/BSD/Apache examples are illustrative. Record this licence in the package inventory |
| FreeType 2.13.3 | [FTL OR GPL-2.0-or-later](https://github.com/freetype/freetype/blob/VER-2-13-3/LICENSE.TXT) | Yes using **FTL**, a BSD-style licence with attribution; do not choose GPL |
| Graphite2 1.3.14 | [LGPL-2.1-or-later OR MPL OR GPL-2.0-or-later](https://github.com/silnrsi/graphite/blob/1.3.14/COPYING) | MPL dependency route allowed; COPYING does not specify an MPL version, so preserve the grant and settle the selected terms before distribution |
| libpng 1.6.43 | [libpng licence](https://github.com/pnggroup/libpng/blob/v1.6.43/LICENSE) | Yes, permissive with notices |
| zlib 1.3.1 | [zlib licence](https://github.com/madler/zlib/blob/v1.3.1/LICENSE) | Yes |
| wasi-libc | [Apache-2.0/LLVM exception, MIT and other retained notices](https://github.com/WebAssembly/wasi-libc/blob/main/LICENSE) | Yes for the build's included permissive code; preserve the notice bundle |
| libc++ / libc++abi / compiler runtime | [Apache-2.0 WITH LLVM exception](https://github.com/llvm/llvm-project/blob/main/libcxx/LICENSE.TXT) and retained third-party notices | Yes, keep the runtime notices |
| Fontconfig / macOS framework bindings | The pinned fork cfg-gates native Fontconfig/CoreText out on wasm32; its WASI font lookup is Rust code | No native Fontconfig/CoreText library linked in this build |
| Rust engine glue and compression | MIT/Apache-2.0 families; flate2 uses its Rust backend on wasm32. Exact lock and metadata inventory recorded | Yes for the reviewed permissive crates; metadata cannot override the inherited GPL/CPL source notices above |
| tecgonic wrapper and WASI fork additions | [MIT](https://github.com/mgilbir/tecgonic/blob/main/LICENSE) | Wrapper yes; its bundled engine still has the restrictions above. No Go runtime is included in this App component |
| SwiftLaTeX official repository | [AGPL-3.0](https://github.com/SwiftLaTeX/SwiftLaTeX/blob/master/LICENSE), confirmed by its own README | **No** for the combined project; a mirror's badge was not the basis |
| BusyTeX upstream build scripts | [MIT, explicitly excluding released binaries](https://github.com/busytex/busytex#license) | Scripts yes; binary **not cleared**, linked TeX Live licences still apply |
| TeXlyre BusyTeX build | [AGPL-3.0](https://github.com/TeXlyre/texlyre-busytex-build/blob/main/LICENSE) | **No** |
| texlive.js, manuels upstream | [GPL-2.0](https://github.com/manuels/texlive.js/blob/master/LICENSE) | **No** |
| CodeMirror 6 | [MIT](https://github.com/codemirror/dev/blob/main/LICENSE) | Yes, retain each used package's notice |
| PDF.js | [Apache-2.0](https://github.com/mozilla/pdf.js/blob/master/LICENSE) | Yes, retain licence/NOTICE and bundled dependency notices |
| Typst | [Apache-2.0](https://github.com/typst/typst/blob/main/LICENSE) | Yes, subject to its dependency/font inventory |
| TeX Live macros/packages | Mixed; e.g. [LaTeX LPPL-1.3c](https://github.com/latex3/latex2e/blob/develop/base/lppl.txt) | Data fetched to compile documents does not automatically relicense an independently written App. Keep each file's terms, source/rename requirements if modified, and avoid assuming the whole bundle is LPPL |
| Fonts | Mixed: [OFL-1.1](https://openfontlicense.org/open-font-license-official-text/), [GUST Font License](https://www.gust.org.pl/projects/e-foundry/licenses/GUST-FONT-LICENSE.txt) for Latin Modern, [Knuth terms](https://ctan.org/license/knuth?lang=en) for original Computer Modern, etc. | Assess the actual font files. OFL generally permits embedding with its conditions; fonts are not all OFL. Keep reserved-name/redistribution notices |

A fetched package is still licensed code/data. Calling it "data" does not
remove redistribution obligations. Distributing a full TeX Live bundle
requires its own inventory. Merely compiling a document with LPPL macros
usually does not put the App or resulting document under LPPL. A component
that *links* GPL engine code is a different case: fetching that component
at runtime does not make it a first-party permissive engine.

## The App Miguel would see

A normal, badge-free App Tab: file tree, CodeMirror 6 editor, PDF.js preview,
compile progress and a useful error list. The published channel has no DEV
badge. The separate dev channel keeps the shape document's development
semantics. Projects live in the App's files; no arbitrary shell escape.
Fonts and packages use the broker and a private, versioned cache.

| Declared action (§4) | Contract |
|---|---|
| `describe_document` | Outline, labels, citation keys, figure files, current revision and compile state. Macro-generated structure is marked uncertain |
| `edit_section` | Stable section/range plus expected document revision; reject a stale edit with a useful conflict |
| `insert_figure` | Add a broker-owned image and source markup with caption/label |
| `add_citation` | Add/update a BibTeX record, then cite its key at a named source position |
| `compile` | Start a compilation job; expose progress, output revision, package fetches and cancellation |
| `get_errors` | `{file, line, message}` and warnings; never invent a filename when the log does not establish one |
| `jump_to` | Source ↔ PDF mapping. Requires exported SyncTeX and a tested position reader, rather than a promise based only on a compiler flag |

Native Tectonic emitted a real `main.synctex.gz` (490 bytes for the article).
The fork driver used by this component leaves SyncTeX disabled and does not
export a mapping file. Source/PDF jumps are therefore **not available in the
measured component**, though the engine has the API. They are T4 work.

The broken-document example is `main.tex:3: Undefined control sequence`,
parsed into `{file: "main.tex", line: 3, message: "Undefined control sequence"}`.
Four parser regressions pass, including an included-file path, an unknown
file and the actual component stderr prefix. Both native and component
broken-document logs produce that structured error. The WASI fork's fatal
`longjmp` path still returns `APP_TRAPPED` to the caller and recycles the
instance; the parent-side parser recovers the file/line diagnostic from
stderr. Wiring it into the App's `get_errors` result is still T1 work.

These are declared UI actions through A2, using `view.state` and
`view.changed`; they are not a second, disconnected MCP document store.
The user and agent edit the same revision. Revision checks matter when both
are typing. Reading an outline or compiler error need not send a screenshot
or a whole TeX log. This can reduce tool calls compared with Overleaf plus
Computer Use; it has **not** been benchmarked against a well-written Overleaf
MCP integration, and no token or reliability advantage is claimed yet.

**Gaps against Overleaf:** no real-time multiplayer editing, track changes,
rich-text mode, template gallery or hosted collaboration service. The measured v33 bundle resolves to `tlextras-2022.0r0.tar`; this is not a
current CTAN installation or proof of compatibility with arbitrary new
packages. Tectonic is not all of TeX Live: BibTeX is in-process, biber and arbitrary shell
programs are not. A view-side Emscripten engine also needs a live view;
headless agent use would depend on A3 and would move compilation authority
out of the backend component.

The measured wrapper also leaves the engine build date at its epoch default;
beamer shows **January 1, 1970**. A real App must supply an explicit build
date through the engine contract. This spike uses flat filenames and a
fixed package subset; arbitrary nested projects and package discovery
remain integration work.

## Alternatives if we keep the rule

- **Typst component:** the clearest first-party engine candidate. It is a
  different language, not a way to compile a user's existing `.tex` project.
- **BusyTeX / SwiftLaTeX in a view Web Worker:** technically plausible
  Emscripten routes, like DuckDB in §7b. No full build was attempted for
  those alternatives. Their engine licences still need clearance; moving
  GPL/AGPL code into the view does not satisfy the first-party rule.
- **Wrapped binary (§7):** let a user install a separately licensed native
  Tectonic or TeX Live engine, and drive it as a terminal/native App. This
  can support existing LaTeX projects, with per-OS packaging and the wrapped
  kind's limits. It is not the portable permissive reference App. An existing
  Overleaf CE server can also be opened as the existing-web-app kind from
  §7; its Node/services/database/TeX stack stays outside the WASM App and
  keeps its own AGPL obligations.

## Bounded PRs, after Apps resumes

| # | PR | Size | Depends on |
|---|---|---|---|
| T0 | Decide the engine distribution/licence route; pin its notices and font/package manifest | S | Owner/legal clearance if Tectonic is wanted |
| T1 | Compiler component, broker package cache, job/progress/cancel and structured errors | M | T0, R3, R6, Rust SDK (or L1) |
| T2 | CodeMirror + PDF.js view, project files, compile button/error jumps | M | T1, A2, L1 or Rust SDK |
| T3 | Declared document actions with revision checks and headless use | M | T2, A2, A3 |
| T4 | SyncTeX mapping and tested source/PDF jumps | S–M | T1, T2 |
| T5 | Real user drills: fresh project, imports, edit/compile/errors, reconnect, DPR1/2 and hosted relay | M | T2–T4 |

Estimate: roughly **3–5 weeks after the prerequisites**, for a single-user
example, excluding collaboration and any engine relicensing/replacement.
This is a planning estimate, not work authorised while Apps is paused.

## Open questions

1. Keep Typst as the first-party typesetting example and offer LaTeX only
   through wrapping? **Recommended**, given the verified licence conflict.
2. If true first-party LaTeX is essential, which acceptable engine licence
   route is available? Rewriting the UI alone is insufficient.
3. Can long compiler jobs use explicit progress/cancel checkpoints or a
   dedicated compiler instance without undermining the App's other actions?
   A synchronous C compiler must not silently inherit relaxed worker limits.
