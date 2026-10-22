#!/usr/bin/env python3
"""MP-08/MP-11: minimal reproducible adjustments to the pinned WASI fork."""
from pathlib import Path
import sys
fork=Path(sys.argv[1])
p=fork/'wasi-deps/build-wasi-deps.sh';s=p.read_text().replace('NPROC="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"','NPROC=4').replace('wasm32-wasi ', 'wasm32-wasip2 ').replace('/usr/share/misc/config.', '${WASI_SDK_PATH}/share/misc/config.')
p.write_text(s)
p=fork/'wasi-deps/toolchain-wasi.cmake';p.write_text(p.read_text().replace('"wasm32-wasi"','"wasm32-wasip2"'))
