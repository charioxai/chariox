#!/bin/bash
# Headless checks only, no helper rebuild/sign/launch or permission request.
set -euo pipefail
cd "$(dirname "$0")"
build=/Users/miguel/.chariox/dev/cumac/build/r10
mkdir -p "$build/modules"
common=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift ProtectedPixels.swift CaptureProtection.swift NativePrivacyPolicy.swift Targets.swift NativeTargets.swift VisionOCR.swift)
check() {
  if [[ $(sysctl -n kern.memorystatus_vm_pressure_level) -ge 4 ]]; then
    printf 'Critical memory pressure; stopping target checks\n' >&2; exit 1
  fi
  swiftc -j2 -parse-as-library -module-cache-path "$build/modules" "${common[@]}" "$1.swift" -o "$build/$1"
  "$build/$1"
}
check M4ProtectionTests
check TargetTests
check PrivacyTests
check OCRTests
common+=(NativeCaptureStream.swift PairedCapture.swift)
check PairedObservationTests
common+=(PhysicalHuman.swift NativeInput.swift PairedNativeInput.swift PairedSession.swift)
check PairedTargetsTests
