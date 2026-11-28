# Documents

Markdown and HTML documents the person and their agents write together, on the wasm runtime.

- `backend/` the Rust backend (`chariox-app-sdk`): `model.rs` the index and its rules, `index.rs` the
  state commits and the change feed, `store.rs` the files in the App's private `data/documents/<id>/`,
  `transfer.rs` import, export and snapshot through the kernel's file broker.
- `src/` the page, built into one self-contained client view; `schemas/` the signed tools and view actions.
- Every edit names the revision it started from; a stale editor is refused, never merged. Ten revisions
  of each document are kept. Files and state keep the layout the earlier runtime used, so an update
  keeps the documents.
- Import: `import_documents` asks the owner for files in the kernel's prompt and waits up to 20 s per call;
  while the owner has not chosen it returns `{pending, importId}` and the caller asks again with that id
  (the kernel holds the request ten minutes), so no call outlasts the kernel's 30 s limit.
- The window hands an unsaved edit to the App (`save_draft`, one draft per document and editor in
  `data/drafts/<id>.<editor>.json`, with the revision it started from) because the view's sandbox keeps nothing
  across a replacement. Each window instance is its own editor with a random name, so two windows on one document
  keep two drafts, and a window that saves or gives up its edit discards its own draft of that document
  (`discard_draft`) and no other. A new window asks `claim_draft` once: it keeps its own draft, otherwise the most
  recently kept draft of another editor moves to it, and a draft whose document was deleted is dropped. It restores
  the edit, so a save after someone else's change is still refused.
- An agent's `delete_document` is an `approval` action: the person confirms it in the Room. The person's own
  delete in the window is one click with the usual confirm.

```sh
cargo test --manifest-path examples/apps/documents/backend/Cargo.toml
node --test examples/apps/documents/test/page.test.mjs
# the real component on a real WASI data directory (build it first, see below)
CHARIOX_DOCUMENTS_COMPONENT=$out/documents.core.wasm TMPDIR=... cargo test --locked -p chariox-app-sdk-tools \
  --features component-drills --test documents-drill
# the signed package (run through the shared builder's cargo slot, output outside the checkout)
examples/apps/documents/scripts/build-package.sh $out
```
