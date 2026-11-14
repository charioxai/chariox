#!/bin/bash
# Builds kernel-browser-font-subset.wasm from the pinned sources below: ./build.sh <work-dir> [output.wasm]
# Needs git, curl, tar and sha256sum; the compiler is the pinned wasi-sdk, so the result does not depend on the host's clang.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); work=$(realpath "${1:?work directory (empty or reused)}"); out=${2:-$here/../../slice-linux-docker/docker/kernel-browser-font-subset.wasm}
HARFBUZZ=35f656a1a2bab548ea7f54a6dd3fba2b09680d14        # harfbuzz/harfbuzz, version 15.0.0
WOFF2=7b67e12355fe7da148db5c19d4261f0a498a9537           # google/woff2, 1.0.2
BROTLI=db24a98aed38a16de78cc05ca15cdd8dc2a0d92d          # google/brotli, the submodule pin of that woff2 commit
WASI_SDK_URL=https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz
WASI_SDK_SHA256=b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4
mkdir -p "$work"; cd "$work"
fetch() { [ -d "$1/.git" ] || { git init -q "$1"; git -C "$1" remote add origin "$2"; }; git -C "$1" fetch -q --depth 1 origin "$3"; git -C "$1" checkout -q --detach FETCH_HEAD; [ "$(git -C "$1" rev-parse HEAD)" = "$3" ]; }
fetch harfbuzz https://github.com/harfbuzz/harfbuzz.git $HARFBUZZ
fetch woff2 https://github.com/google/woff2.git $WOFF2
fetch brotli https://github.com/google/brotli.git $BROTLI
if [ ! -x wasi-sdk/bin/clang++ ]; then
  curl -sSL -o wasi-sdk.tar.gz "$WASI_SDK_URL"; echo "$WASI_SDK_SHA256  wasi-sdk.tar.gz" | sha256sum -c -
  mkdir -p wasi-sdk; tar -xzf wasi-sdk.tar.gz -C wasi-sdk --strip-components=1
fi
sdk=$PWD/wasi-sdk
# HB_TINY drops what a subsetter does not need; the override keeps the tables of CFF, variable and colour fonts.
cat > hb-config-override.h <<'H'
#undef HB_NO_CFF
#undef HB_NO_FACE_COLLECT_UNICODES
#undef HB_NO_OT_FONT_CFF
#undef HB_NO_COLOR
#undef HB_NO_VAR
#undef HB_NO_AVAR2
#undef HB_NO_CUBIC_GLYF
#undef HB_NO_VAR_COMPOSITES
#undef HB_NO_NAME
#undef HB_NO_METRICS
#undef HB_NO_LAYOUT_FEATURE_PARAMS
#undef HB_NO_LAYOUT_RARELY_USED
#undef HB_NO_LAYOUT_UNUSED
#undef HB_NO_SUBSET_CFF
#undef HB_NO_SUBSET_LAYOUT
H
flags=(--target=wasm32-wasip1 -Oz -flto -fno-exceptions -fno-rtti -fno-threadsafe-statics -ffunction-sections -fdata-sections -D__STDC_FORMAT_MACROS)
inc=(-Iharfbuzz/src -Iwoff2/src -Iwoff2/include -Ibrotli/c/include)
mkdir -p obj
for f in brotli/c/common/*.c brotli/c/dec/*.c brotli/c/enc/*.c; do $sdk/bin/clang "${flags[@]}" -Ibrotli/c/include -c "$f" -o obj/$(basename $(dirname $f))-$(basename $f).o; done
for f in table_tags variable_length woff2_common woff2_dec woff2_out font glyph normalize transform woff2_enc; do $sdk/bin/clang++ "${flags[@]}" -std=c++17 "${inc[@]}" -c woff2/src/$f.cc -o obj/woff2-$f.o; done
$sdk/bin/clang++ "${flags[@]}" -std=c++17 "${inc[@]}" -DHB_TINY -DHB_USE_INTERNAL_QSORT -DHB_CONFIG_OVERRIDE_H=\"$PWD/hb-config-override.h\" -c harfbuzz/src/harfbuzz-subset.cc -o obj/harfbuzz-subset.o
$sdk/bin/clang++ "${flags[@]}" -std=c++17 "${inc[@]}" -c "$here/subset.cc" -o obj/subset.o
$sdk/bin/clang++ "${flags[@]}" -mexec-model=reactor -Wl,--no-entry -Wl,--gc-sections -Wl,--strip-all -Wl,--export=font_alloc -Wl,--export=font_free -Wl,--export=font_subset -Wl,--export=_initialize -Wl,--initial-memory=1048576 -Wl,--max-memory=268435456 -Wl,--export-memory obj/*.o -o "$out"
sha256sum "$out"
