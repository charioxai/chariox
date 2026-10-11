#!/bin/bash
# No helper build, signing, launch, permission API or desktop pixels.
set -euo pipefail
cd "$(dirname "$0")"
build=/Users/miguel/.chariox/dev/cumac/build/r11
mkdir -p "$build/modules"
flags=(-j2 -parse-as-library -module-cache-path "$build/modules")
common=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift)
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift CodecTests.swift -o "$build/codec-tests"
"$build/codec-tests"
swiftc "${flags[@]}" CodecDecodeTests.swift -o "$build/codec-decode-tests"
"$build/codec-decode-tests" /Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r11-20261011/public-protected.h264
