# Excalidraw reference App (MP-08 / MP-09 / MP-10 / MP-11)

This first-party Apps v2 example embeds Excalidraw **0.18.1** at
`a2ec2889babf7d2295469c6d90ebe77fae57df84`. React and esbuild are build/view
dependencies; the backend is a portable Rust component using the L1 SDK.
Rust supplies private-file reads through R6's `data` preopen without adding a
host interface or a second bridge.

The kernel installs the signed package and supplies `window.chariox` to its
normal App view. The view registers exactly `schemas/ui-actions.json` through
A1's `chariox.action`, `view.state` and `view.changed`. No developer server,
remote CDN or provider SDK is involved.

## Build and sign

Use the repository's pinned Node/pnpm and Rust 1.99.1. Follow
[the component build prerequisites](../../../packages/app-sdk-tools/README.md).
Every Cargo command on a shared builder must run through its Cargo slot wrapper;
copy tools out of a shared target before releasing the slot. Bound Node builds
to 4 GiB on the lane builder.

```sh
pnpm --dir examples/apps/excalidraw install --ignore-workspace --frozen-lockfile
cargo build --locked -p chariox-app-sdk-tools -p chariox-app-package --bins
export CHARIOX_APP_COMPONENT_CHECK="$PWD/target/debug/chariox-app-component"
export TMPDIR="$HOME/.chariox/dev/excalidraw-build/tmp"
mkdir -p "$TMPDIR"
app_out="$HOME/.chariox/dev/excalidraw-build/bundle"
node examples/apps/excalidraw/scripts/build-view.mjs "$app_out"
mkdir -p "$app_out/runtime"
python3 packages/app-sdk-tools/build-rust.py examples/apps/excalidraw/backend/Cargo.toml "$app_out/runtime/app.wasm"
```

Use a fresh output directory outside the checkout. The component builder embeds
the Rust, SDK, standard-library and WASI notice inventories in `app.wasm`.
`view-build.json` records the view's actual esbuild inputs.

Generate a local development publisher with `chariox app keygen` (or
`chariox-app-package keygen`), keeping its private file outside both source and
bundle. Copy `app.json` outside the checkout and replace its `publisher` with
the generated public enrollment file's `publisher`. Then use the ordinary
`chariox app pack --bundle … --manifest … --key … --output …` and
`chariox app validate ARCHIVE --trust PUBLIC` commands. The standalone package
tool additionally requires `--kernel-protocol N`; the Chariox CLI supplies it.
V2 pack and validate use the same v2 verifier as the installer, including main
component admission and signed action/schema checks. A local publisher file
does not enroll trust: use the terminal's `/app publisher enroll` and normal
install approval flow.

## Diagram and file behavior

The signed actions add labelled shapes/text/arrows, move or resize shapes, delete,
group, connect and select elements; read a paginated summary; and save, open or
export files. All operate on Excalidraw's own scene store. Its native toolbar,
pointer editing and undo history remain available. File menu entries and native
Open/Save shortcuts call the same broker functions as declared actions. Dropped
files also request a kernel file grant.
Opening a diagram keeps the previous canvas in native Undo history. Use Undo
before closing or reloading the view to restore it; the restored scene auto-saves.

The current scene auto-saves after 300 ms without an edit, with an optimistic
state version check. Explicit private files use `files.atomic_replace`; imports
use `host.pick_file` and `files.import`, and exports use `files.export`. Inputs
are filenames, never host paths. Only WASI's installation-private `data` preopen
is read directly. Named private saves survive worker/view restarts independently
of the current scene. Save a copy (also Ctrl+S and Ctrl+Shift+S) offers a new,
uniquely named `.excalidraw` copy through the kernel's file-save interaction.
In a text terminal, use its displayed `/app file save OPERATION "PATH"` command
to keep the offered file, then choose that file with Open to restore it. The
current Web interaction only offers Decline; use the text terminal to save it.
An agent can still save/open an explicitly
named private file; a repeated explicit name replaces that file.
A concurrent-view conflict is shown once in a dismissible toast. Use Save a
copy and keep the offered file before reloading, then Open it to recover edits.
Reloading clears the automatic-save error and loads the latest durable scene.

`scene_summary` reads actual canvas elements, with IDs, geometry, labels,
selection, groups and arrow bindings. Follow `nextOffset` until null. The state
snapshot uses the same summary and stays inside A1's 64 KiB limit; long text
and free-draw point lists explicitly report truncation. Supported inline scenes
and text exports are at most 192 KiB; PNG exports are at most 144 KiB to leave
room for base64 within the broker's JSON envelope. Larger diagrams fail clearly
and can be split; they are never silently truncated on disk.
The scene budget includes embedded images in imported diagrams. New image
insertion is disabled; Open accepts Excalidraw/JSON through the broker picker.
The UI stays within the browser host's 8 MiB asset budget. Mermaid source is
pasted as plain text; conversion is omitted. Excalifont CJK text uses the
browser's local fallback font instead of bundling Xiaolai's 12 MiB glyph set.
The build checks the exact upstream chunk hash before removing Xiaolai's
downloadable face registration; the fallback family and metrics remain.

## Licenses and assets

`licenses.json` is the reviewed allowlist for the complete installed dependency
closure, lockfiles, retained notices and upstream production assets. Every build
checks it and checks actual bundled npm inputs. An added/changed dependency,
asset or notice fails the build until reviewed. Do not regenerate the allowlist
as a build step. Exact npm integrity pins are in `pnpm-lock.yaml`; additional
source revisions and notice origins are in `licenses/sources.json` and
`licenses/supplemental.json`. Build-only optional esbuild binaries are platform
specific; another platform's binary must be reviewed before building there.

The shipped code uses MIT, ISC, BSD, Apache, 0BSD, CC0 and public-domain licenses.
For `MPL-2.0 OR Apache-2.0`, the selected license is Apache-2.0. Pako's combined
MIT/Zlib notices are both retained. Published packages that omit license files
have pinned supplemental notices, including the WorkOS MIT notice for Radix.

Font notices cover Assistant, Cascadia, Excalifont, Lilita, Nunito, Virgil and
Xiaolai under OFL 1.1, and ComicShanns under MIT. Helvetica and Emoji use local
system fonts and distribute no font binaries. **Upstream Liberation 1.05 is
GPL with a font exception and is excluded.** The build replaces it with the
included OFL Liberation Sans 2.1.5 asset from Debian's checksummed 2.1.5-1
package. Its conversion, source revision, copyright and OFL text are retained.
The distribution test checks that the old bytes cannot reappear. The bundle
contains the full dependency/font notices under `licenses/`. No GPL/AGPL asset
or code is distributed.

## Regression checks and live acceptance

```sh
cargo test --locked --manifest-path examples/apps/excalidraw/backend/Cargo.toml
CHARIOX_EXCALIDRAW_BUNDLE="$app_out" \
CHARIOX_EXCALIDRAW_EVIDENCE="$HOME/.chariox/dev/excalidraw-build/evidence" \
  node --test --test-concurrency=1 examples/apps/excalidraw/test/*.test.mjs
```

The browser regression uses real Chromium, Excalidraw and A1 at DPR 1 and 2.
It draws through native pointer input, invokes declarations, verifies the
snapshot, moves/connects/groups/deletes, exercises brokered shortcuts, reloads
saved state and independently checks JSON, SVG text and PNG dimensions. Its
storage adapter is a fixture. These checks do not establish S1-04 or S1-12;
those require the real installed App, kernel, client, provider and file broker
drills on the integrated R4/R6 validation head. MP-09 shutdown policy is unchanged.
