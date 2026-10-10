#!/bin/bash
# Headless paired reply check, never builds or signs a helper bundle.
set -euo pipefail
cd "$(dirname "$0")"
build=/Users/miguel/.chariox/dev/cumac/build/r9/refusals
mkdir -p "$build/modules"
if [[ $(sysctl -n kern.memorystatus_vm_pressure_level) -ge 4 ]]; then exit 1; fi
sources=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift)
for file in CaptureDamage CaptureAttachment ProtectedPixels NativeCaptureStream CaptureProtection PairedCapture PhysicalHuman NativeInput PairedNativeInput; do
  if [[ -f $file.swift ]]; then sources+=("$file.swift"); fi
done
swiftc -j2 -parse-as-library -module-cache-path "$build/modules" "${sources[@]}" PairedSession.swift PairedRefusalTests.swift -o "$build/paired-refusals"
"$build/paired-refusals"
