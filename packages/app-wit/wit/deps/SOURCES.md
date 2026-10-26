# Vendored WASI definitions

- `*-0.3.0/package.wit`: copied byte-for-byte from `spikes/wasm/wit-full/deps` at
  `fea6b0ff8`; those definitions came from `wasmtime-wasi` / `wasmtime-wasi-http`
  50.0.0 (WASI 0.3.0), as recorded in the spike DESIGN.md §2.
- `*-0.2.0/package.wit`: `wasmtime-wasi` 20.0.2, `wit/deps/*` (WASI 0.2.0), obtained
  from its published crates.io source archive. Files within each package are
  sorted by filename and concatenated; repeated package headers are removed.
  All upstream interfaces and documentation remain unchanged.
- `LICENSE`: the upstream Apache-2.0 WITH LLVM-exception licence, retained from
  `wasmtime-wasi` 20.0.2. WASI 0.3 definitions have the same upstream licence.
- `SHA256.json`: hashes of the vendored files after the documented flattening.

Unused upstream declarations, including sockets and complete CLI worlds, are
needed to resolve the upstream packages. They are not host policy: only the
interfaces imported by `../admission.wit` and `../app.wit` are admitted.
