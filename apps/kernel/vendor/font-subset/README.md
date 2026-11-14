# Font subsetter (wasm)

`apps/kernel/slice-linux-docker/docker/kernel-browser-font-subset.wasm` is the module the Browser mirror uses to send a
large web font as the glyphs the page can show (`kernel-browser-font-subset.mjs`; it runs in a worker thread). It decodes
WOFF2, TrueType or OpenType, keeps the requested code points (and every private-use code point) with HarfBuzz's
`hb-subset`, and writes WOFF2 again. `subset.cc` is the whole of our own code; `build.sh` builds the module.

| Part | Source | Pinned at | Licence |
| --- | --- | --- | --- |
| HarfBuzz (hb-subset) | https://github.com/harfbuzz/harfbuzz | `35f656a1a2bab548ea7f54a6dd3fba2b09680d14` (15.0.0) | "Old MIT" (`licenses/harfbuzz-COPYING`) |
| WOFF2 codec | https://github.com/google/woff2 | `7b67e12355fe7da148db5c19d4261f0a498a9537` (1.0.2) | MIT (`licenses/woff2-LICENSE`) |
| Brotli (used by the codec) | https://github.com/google/brotli | `db24a98aed38a16de78cc05ca15cdd8dc2a0d92d` (the submodule pin of that woff2 commit) | MIT (`licenses/brotli-LICENSE`) |
| wasi-sdk 34 (compiler, wasi-libc, libc++ linked in statically) | https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34 | tarball sha256 `b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4` (wasi-libc `2e6fb9d8ee0c`, llvm `895aa2c896ad`) | wasi-libc: MIT, Apache-2.0 and Apache-2.0 with LLVM exception (`licenses/wasi-libc-LICENSE*`); libc++: Apache-2.0 with LLVM exception (`licenses/llvm-libcxx-LICENSE.TXT`) |

Rebuild: `./build.sh <empty work directory>` (needs git, curl and network; it fetches the pins above, checks the wasi-sdk
checksum and writes the module next to `kernel-browser-font-subset.mjs`). The build is `-Oz` with LTO and no exceptions;
HarfBuzz uses the `HB_TINY` configuration with the CFF, variation, colour and layout subsetting kept. The result
(`sha256` printed by the script) is committed, so a kernel build needs no toolchain. Changing a pin changes the module:
update the table, the licences and the module together.

The module imports five WASI calls (`fd_close`, `fd_seek`, `fd_write`, `proc_exit`, `random_get`); the worker answers them
without a file, socket or clock.
