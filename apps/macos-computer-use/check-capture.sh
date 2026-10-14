#!/bin/bash
# Focused headless executables only. Never builds/signs the helper bundle.
set -euo pipefail
cd "$(dirname "$0")"
build=/Users/miguel/.chariox/dev/cumac/build/r8-tests
mkdir -p "$build/modules"
common=(Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift Pairing.swift)
flags=(-j2 -parse-as-library -module-cache-path "$build/modules")
check_pressure() {
  if [[ $(sysctl -n kern.memorystatus_vm_pressure_level) -ge 4 ]]; then
    printf 'Critical memory pressure; stopping capture checks\n' >&2; exit 1
  fi
}
check_pressure
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift AttachmentTests.swift -o "$build/attachment-tests"
"$build/attachment-tests"
check_pressure
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift ProtectedPixels.swift CaptureTests.swift -o "$build/capture-tests"
"$build/capture-tests"
check_pressure
swiftc "${flags[@]}" "${common[@]}" NativeCaptureStream.swift StreamTests.swift -o "$build/stream-tests"
"$build/stream-tests"
check_pressure
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift ProtectedPixels.swift NativeCaptureStream.swift CaptureProtection.swift NativePrivacyPolicy.swift Targets.swift NativeTargets.swift VisionOCR.swift PairedCapture.swift PhysicalHuman.swift NativeInput.swift PairedNativeInput.swift PairedSession.swift ChannelTests.swift -o "$build/channel-tests"
"$build/channel-tests"
check_pressure
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift ProtectedPixels.swift CaptureReplay.swift -o "$build/capture-replay"

check_pressure
swiftc "${flags[@]}" Policy.swift CaptureProtection.swift CaptureProtectionTests.swift -o "$build/protection-tests"
"$build/protection-tests"

check_pressure
swiftc "${flags[@]}" "${common[@]}" CaptureDamage.swift CaptureAttachment.swift MacVideoEncoder.swift ProtectedPixels.swift NativeCaptureStream.swift CaptureProtection.swift NativePrivacyPolicy.swift Targets.swift NativeTargets.swift VisionOCR.swift PairedCapture.swift PhysicalHuman.swift NativeInput.swift PairedNativeInput.swift PairedSession.swift PairedFailureTests.swift -o "$build/paired-failure-tests"
"$build/paired-failure-tests"
