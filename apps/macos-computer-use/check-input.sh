#!/bin/bash
# Headless checks only. No bundle build, signing, permissions or event posting.
set -euo pipefail
cd "$(dirname "$0")"
build=/Users/miguel/.chariox/dev/cumac/build/r9
mkdir -p "$build/modules"
common=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift PhysicalHuman.swift NativeInput.swift)
check() {
  if [[ $(sysctl -n kern.memorystatus_vm_pressure_level) -ge 4 ]]; then
    printf 'Critical memory pressure; stopping input checks\n' >&2; exit 1
  fi
  swiftc -j2 -parse-as-library -module-cache-path "$build/modules" "${common[@]}" "$1.swift" -o "$build/$1"
  "$build/$1"
}
check M3ReleaseTests
check PhysicalHumanTests
check NativeInputTests

common+=(CaptureDamage.swift CaptureAttachment.swift ProtectedPixels.swift NativeCaptureStream.swift CaptureProtection.swift PairedCapture.swift PairedNativeInput.swift PairedSession.swift)
check PairedNativeInputTests
check PairedRefusalTests
