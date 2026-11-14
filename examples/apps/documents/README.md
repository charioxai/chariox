# Documents

Markdown and HTML documents the person and their agents write together, on the wasm runtime.

- `backend/` the Rust backend (`chariox-app-sdk`): `model.rs` the index and its rules, `index.rs` the
  state commits and the change feed, `store.rs` the files in the App's private `data/documents/<id>/`,
  `transfer.rs` import, export and snapshot through the kernel's file broker.
- `src/` the page, built into one self-contained client view; `schemas/` the signed tools and view actions.
- Every edit names the revision it started from; a stale editor is refused, never merged. Ten revisions
  of each document are kept. Files and state keep the layout the earlier runtime used, so an update
  keeps the documents.

```sh
cargo test --manifest-path examples/apps/documents/backend/Cargo.toml
node --test examples/apps/documents/test/page.test.mjs
# the real component on a real WASI data directory (build it first, see below)
CHARIOX_DOCUMENTS_COMPONENT=$out/documents.core.wasm TMPDIR=... cargo test --locked -p chariox-app-sdk-tools \
  --features component-drills --test documents-drill
# the signed package (run through the shared builder's cargo slot, output outside the checkout)
examples/apps/documents/scripts/build-package.sh $out
```
