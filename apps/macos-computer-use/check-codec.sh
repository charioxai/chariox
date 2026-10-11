#!/bin/bash
# No helper build, signing, launch, permission API or desktop pixels.
set -euo pipefail
cd "$(dirname "$0")"
build=${CUMAC_BUILD_DIR:-/Users/miguel/.chariox/dev/cumac/build/codec-tests}
evidence=${CUMAC_EVIDENCE_DIR:-/Users/miguel/.codex/evidence/browser-computer-use-macos/codec-tests}
mkdir -p "$build/modules" "$evidence"
artifact="$evidence/public-protected.h264"
flags=(-j2 -parse-as-library -module-cache-path "$build/modules")
common=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift)
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift CodecTests.swift -o "$build/codec-tests"
"$build/codec-tests" "$artifact"
swiftc "${flags[@]}" CodecDecodeTests.swift -o "$build/codec-decode-tests"
"$build/codec-decode-tests" "$artifact"
