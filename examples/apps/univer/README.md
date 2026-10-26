# Univer Sheets reference App

MP-08 / MP-09 / MP-10 / MP-11, Apps v2 X2 / S1-12 (Univer half).

This App uses the Apache-2.0 Univer **1.2.0** core and sort presets. The real
editor handles cells, formulas, sorting, row/column edits, undo, sheets, menus
and canvas rendering. Its existing toolbar and menu components provide Files
→ Save workbook, Open CSV, Open workbook, Export CSV and Export workbook.
No Chariox shell UI or framework is added.

The browser registers the exact signed declarations in
`schemas/ui-actions.json` through A1/A2's `chariox.action`, `chariox.view.state`
and `chariox.view.changed`. Human file-menu callbacks use the same functions.
The snapshot reports the actual workbook, used sheet dimensions, selection,
save version, unsaved changes and a bounded A1:H10 preview. `read_range` returns
both calculated values and formulas. Formula results are awaited before reads,
saves and exports; sorting uses Univer's reference-aware sort command.

The L1 **Rust SDK** backend provides version-checked state and brokered files.
Rust is used because the worker's WASI `data` preopen can read an imported file
without introducing a new host read API or a Node backend. `files.import`,
`files.atomic-replace`, `files.export` and `host.pick-file` retain the kernel's
normal capability and user-selection flows. The backend accepts plain filenames
only and reads only `data/<validated-name>` under R6's installation preopens.
No host paths, ambient network capability, provider SDK or custom bridge exists.

Save is explicit; save or export changes before closing the view. Native browser
confirmation is disabled by the App sandbox. A failed or conflicting save retains the current workbook;
export it before reopening another client's saved copy. Opening a file replaces
the open workbook and marks it unsaved until Save workbook succeeds. Edits made
while an import is reading cause that import to refuse before replacement.

## Build and checks

Use the repository's pinned Rust 1.99.1 and L1 toolchain setup. This example has
its own exact pnpm and Cargo locks, independent of the workspace install.
Choose bundle and scratch directories outside the checkout.

```sh
pnpm --dir examples/apps/univer --ignore-workspace install --frozen-lockfile
node examples/apps/univer/scripts/build-view.mjs "$BUNDLE"
# Inside a Cargo slot; see packages/app-sdk-tools/README.md for the checker:
bash examples/apps/univer/scripts/build-backend.sh "$BUNDLE"
node --test --test-concurrency=1 examples/apps/univer/test/*.test.mjs
cargo test --locked --manifest-path examples/apps/univer/backend/Cargo.toml
```

On b3 run Node under `systemd-run --scope -p MemoryMax=4G -p MemorySwapMax=0`
with `NODE_OPTIONS=--max-old-space-size=3072`, and every Cargo operation through
`agents/cargo-slot.sh`. Do not run the browser as root.
`scripts/drill-view.mjs <bundle> <evidence> <browser-modules-directory>` drives
the actual Univer bundle through the production `KernelBrowserHost` and A2
bridge. Set disposable `CHARIOX_HOME`, `TMPDIR` and XDG config/cache directories.
It checks registration agreement, CSV, formulas after sort, dimension edits,
state, JSON/CSV export and reopen, and captures canvas/menu screenshots at
DPR1 and DPR2. Its backend is a fixture: this is supplementary regression
coverage, **not** install, worker containment, provider, hosted Web or MP
acceptance. Those require the merged R4/R6 validation head and real clients.

The emitted bundle includes licence texts, inventory and the build metafile.
Register with the normal publisher/enrollment flow; `developer-1` in `app.json`
is a placeholder for that publisher's key ID, not a shipped signing identity.

## Supported files and limits

- CSV: UTF-8 (optional BOM), comma separated, CRLF/LF/CR records, quoted commas,
  quotes and newlines. Canonical numbers become numeric cells; unsafe integers,
  leading-zero identifiers, dates and formula-looking CSV text remain literal.
  CSV export contains calculated values; formulas are preserved by JSON.
- JSON: `.univer.json`, `format: chariox-univer-workbook`, `version: 1`, with the
  full validated Univer snapshot. Styles, formulas, values and sheet metadata
  round-trip. This is an App format, not an Excel file.
- Workbooks: at most 192 KiB of snapshot JSON, 16 sheets, 10,000 rows and 256
  columns per sheet. Range actions and a single CSV export cover at most 1,000
  cells. These limits fit the existing state and A2 result budgets; operations
  above them fail explicitly. The CSV export menu asks for an optional A1 range;
  use selected ranges or split a larger workbook. The CSV byte cap also applies
  to input, but the expanded workbook must independently fit its snapshot cap.
- XLSX: **NOT SHIPPED.** See [LICENSING.md](LICENSING.md). There is no XLSX menu,
  extension claim, paid import/export dependency or collaboration feature.

## Licence gate

`scripts/licenses.mjs` compares all installed packages, complete source hashes,
exact dependency/lock hashes, notices and asset lists with `licenses.json`.
Unknown or changed dependencies fail the build. Pro packages in the install,
lock or emitted inputs fail. Only the reviewed permissive licence set is
admitted. Inventory regeneration is an explicit review step, never part of a
build: `--record-reviewed-inventory` records the already reviewed installation.
See [LICENSING.md](LICENSING.md) for retained notices and build modifications.
