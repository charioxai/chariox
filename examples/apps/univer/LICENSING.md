# Licence inventory and format decision

MP-08 / MP-09 / MP-10 / MP-11 — X2, reviewed 2026-10-26.

The App's own code is MIT. `licenses.json` is the complete pinned installed
inventory (130 npm packages on Linux x86-64); `pnpm-lock.yaml` retains exact
transitive versions and archive integrity. Its reviewed licences are MIT,
Apache-2.0, BSD-3-Clause, ISC and 0BSD. No GPL, AGPL, MPL or Univer Pro package
is selected. The Rust lock is pinned by the same inventory. L1's backend
builder inspects all Rust dependencies, embeds their licence texts, and embeds
the pinned Rust/WASI libc/compiler-rt/libunwind notices in `chariox:licenses`.
This portable component does not distribute L1's native compiler binaries.

The build retains upstream copyrights and licence/NOTICE files in
`THIRD_PARTY_NOTICES.txt`, including three notices omitted from npm archives:
esbuild's Linux executable (MIT), react-remove-scroll-bar (MIT) and franc-min
(MIT). Their pinned upstream source URLs and SHA-256 digests are in
`licenses/supplemental.json`; their full texts are in the same directory.
unicount and ot-json1 carry their ISC notices in README; ot-text-unicode declares
ISC in package metadata but its README supplies an MIT grant. Both declarations
and the complete original README notice are retained without rewriting them.

Univer's selected core, render, formula, sort and UI code is Apache-2.0, with
its supplied licence texts retained. No paid Pro import/export, collaboration,
font, image or codec is fetched. There are no distributed App font/image assets;
the editor uses system fonts and its packaged icon code. The five assets listed
for playwright-core are development-tool assets and are not emitted in the App
view. The generated bundle metafile records the exact emitted inputs.

To stay within the existing 8 MiB view-asset budget, the build changes Univer
1.2.0's `engine-render/lib/es/index.js` document `PATTERN_LOADERS` table to retain
only the upstream `en-us` loader. This limits optional **document hyphenation**;
it does not remove Unicode cell text or formula functionality. An exact source
inventory and a checked loader shape guard the transformation. The signed
bundle includes `UNIVER_MODIFICATIONS.txt`, naming the modified file and change
as Apache-2.0 requires. No other dependency source is patched.

## XLSX: NOT SHIPPED

Candidates investigated:

| Library | Upstream licence | Assessment |
| --- | --- | --- |
| [ExcelJS 4.4.0](https://github.com/exceljs/exceljs/blob/master/package.json) | MIT | Permissive candidate for separate XLSX work; requires a Univer conversion layer and independent formula/style/date round-trip evidence. |
| [SheetJS Community Edition](https://docs.sheetjs.com/docs/miscellany/license/) | Apache-2.0 | Permissive CE candidate, separate from SheetJS Pro; requires an independently pinned distribution, full dependency review and conversion/round-trip tests. |
| [xlsx-populate 1.21.0](https://github.com/dtjohnson/xlsx-populate/blob/master/package.json) | MIT | Permissive browser/Node candidate, with the same unimplemented conversion and independent-reader obligations. |

**Choice: none.** Permissively licensed candidates exist; this App does not yet
have a verified Excel/Univer mapping or an independent-reader XLSX round trip.
Calling JSON or CSV “XLSX support” would therefore be incorrect. Launch exports
are CSV values and the tested native JSON workbook. XLSX remains unselected and
unshipped until its distinct dependency review and independent round-trip drill
are implemented. Univer's Apache core licence grants no rights to Pro exports.
