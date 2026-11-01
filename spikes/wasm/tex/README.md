# MP-08 / MP-10 / MP-11 — LaTeX compiler research

Read [TEX_APP.md](TEX_APP.md) for the verdict. Apps implementation remains
paused. These scripts never build/start the product kernel or inspect App
state. PDFs, captures, caches and toolchains belong outside the repository.

Base: `wasmspike/r3`, `5c4d888b9347a93575a8412244b29650c9222799`.
Host: b3, Ryzen 9 9950X, 32 logical CPUs, 125 GiB RAM. Shared host; compiler
numbers do not count as MP-10 UI or quiet-host timing acceptance.

## Reproduce

Use `/var/tmp/texspike` for disposable sources/toolchains and
`/root/.chariox/dev/texspike` for state, with its `tmp` on disk. Set
`CHARIOX_HOME` before any product tool (none is needed for this spike).
Do not remove another lane's files or the shared cargo targets. `component-runs.py` resumes existing rows; a failed
row remains a failure. It must be archived explicitly before a deliberate
rerun, so resuming cannot silently erase a bad attempt.

1. Download wasi-sdk 33.0 (`wasi-sdk-33.0-x86_64-linux.tar.gz`), wasm-tools
   1.262.0, native Tectonic 0.17.0 (musl) and Typst 0.15.1 (musl) from their
   official GitHub releases. Extract under `/var/tmp/texspike/tools`.
   Install Rust 1.99.1, minimal + `wasm32-wasip2`, with
   `RUSTUP_HOME=/var/tmp/texspike/rustup`. The evidence `tools.log` and
   `identities.json` record the release URLs and versions used.
2. Clone `https://github.com/mgilbir/tectonic.git` at
   `d1a46a5d127ea40c6a1afcb45e91bb3d7c1f5e47`, update only the
   `crates/bridge_harfbuzz/harfbuzz` submodule, and run
   `python3 prepare-deps.py /var/tmp/texspike/tectonic`.
3. Build its C/C++ dependencies with
   `CMAKE_POLICY_VERSION_MINIMUM=3.5`,
   `WASI_SDK_PATH=/var/tmp/texspike/tools/wasi-sdk-33.0-x86_64-linux`,
   `TMPDIR=/root/.chariox/dev/texspike/tmp`, then
   `bash /var/tmp/texspike/tectonic/wasi-deps/build-wasi-deps.sh`.
   This uses at most four jobs. The policy setting handles CMake 4; SDK
   copies of `config.sub`/`config.guess` replace absent host copies.
4. `python3 prepare-guest.py /var/tmp/texspike` generates a standalone
   Cargo manifest and copies the fork's WASI driver into scratch. It rewrites
   its cache/font roots to the two private preopens. No upstream engine
   code or binary is committed here. Export the Rust/Cargo paths specified
   by the builder rules and run **through `cargo-slot.sh`**:
   `spikes/wasm/tex/build.sh`. The shared target is mandatory on b3; copy the
   exact output immediately, as the script does. The new toolchain and
   private output copies live under `/var/tmp/texspike`, not a private target.
5. Also through `cargo-slot.sh`, build `chariox-wasm-worker-async` and
   `chariox-wasm-harness` from `spikes/wasm/Cargo.toml`, `--release --locked`.
   Touch these exact spike sources first, copy the binaries immediately to
   `/var/tmp/texspike/bin/{worker,harness}`, record their SHA-256 and inspect
   their embedded `/root/work/` paths. No kernel build is needed.
6. `worker check app.wasm`, then
   `worker precompile app.wasm app.cwasm`. Put the AOT artifact at
   `/var/tmp/texspike/spike/artifacts/wasm-rust-async/app.cwasm`; create the
   sibling `runtime` directory for the existing harness launcher.
7. Generate the corpus with `make-documents.py <state>/documents`. Native
   Tectonic preparation of hello/paper/beamer discovers required support
   files in its v33 cache. The spike freezes that **union subset**, including
   format inputs, fonts, paper and beamer packages (rather than downloading
   all TeX Live). The index is the native cache's `.index` file. This is a
   finite corpus manifest, not a general on-demand package resolver.
8. Create each `<state>/<document>.json` as
   `{"main":"main.tex","files":{"main.tex":[byte,...],...},"resources":[name,...]}`.
   Files include the PNG and `.bib`; do not seed generated PDF/aux files.
   Resources are the flat filenames in the native bundle data directory.
   They are fetched again over HTTPS; the proxy never reads cached bodies.
9. Start `bundle-proxy.py --index <v33.index> --port-file <state>/proxy.port
   --log <evidence>/bundle-http.jsonl`. It records its exact PID next to the
   port file. This is the **parent-side origin for the existing harness
   broker**, which admits `https://spike-origin.example/`. Each file maps to
   a real byte-range request to the immutable v33 archive. The proxy retains
   one HTTPS connection, not package bodies. Its log records every upstream
   byte count/range and resolved public URL. A default urllib User-Agent
   received HTTP 403; the explicit Tectonic User-Agent worked, as did curl.
10. `component-runs.py --state <state> --root /var/tmp/texspike --evidence
    <evidence>` runs three cold App instances per document under bubblewrap,
    then a warm compile and a real one-line edit on each instance. For an
    additional process-tree limit, pass `--cgroup-mib 512` with writable
    cgroup v2. The 512 MiB linear-memory cap is always set. Cold compilation
    includes `prepare_resources` batches of 32 files plus format creation;
    batching stays inside the existing 30-second per-call admission limit.
    Warm rows skip HTTP, keep `latex.fmt` and reuse feedback files.
11. `native-runs.py --state <state> --evidence <evidence> --tectonic <binary>
    --typst <binary>` records three cold/warm/edit series for the native
    comparators. Every Tectonic cold cell has its own empty XDG cache; this
    includes actual default-package downloads, unlike a warm-host format
    benchmark. Every child uses `/usr/bin/time -v` and a 300-second timeout.
12. Inspect nested compiler outcomes as well as launcher exit codes. One
    beamer cold attempt received HTTP 200 instead of 206 for `umsb.fd`; it
    was preserved as `component-beamer-3-range-failure.*`, with its complete
    row in `component-failed-attempts.json`. The proxy connection was
    restarted and a fresh cold cell was run once; this is a reported retry,
    not a discarded success sample.
13. Run `test-errors.py`; pipe actual compiler stderr into `parse-errors.py`.
    Inspect PDFs and render representative pages outside git. Remove only
    this run's processes by recorded, validated exact PID, cgroups and
    scratch after preserving evidence.

The unmodified worker's missing-preopen RED result is retained externally.
The normal 1-second CPU-slice and the explicit 30-second compiler-slice cells
must be labelled separately. Increasing a spike limit is not a product fix
or permission to give every App a longer uninterruptible slice.

## Evidence

`/root/.codex/evidence/texspike/`: upstream and as-built licences; exact
identities, build logs, component WIT and hashes; per-run JSON/stderr;
real upstream bundle ranges; native command logs and peak RSS; broken-source
structured errors; PDFs/page renders and resource/cleanup receipts.
The compact committed `results.json` contains measurements, not runtime state.
