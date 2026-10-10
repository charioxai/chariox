#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
build=${CUMAC_BUILD_DIR:-"$HOME/.chariox/dev/cumac/build"}
helper_bundle="Chariox Computer Helper.app"
case "${1:-}" in
  --helper-recheck) helper_bundle="Chariox Computer Helper Recheck.app" ;;
  "") ;;
  *) printf 'Unknown build option\n' >&2; exit 1 ;;
esac
mkdir -p "$build/modules"
arch=$(uname -m)
flags=(-j2 -parse-as-library -target "$arch-apple-macosx14.0" -module-cache-path "$build/modules")
swiftc "${flags[@]}" Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift PointerInputTests.swift Pairing.swift Tests.swift -o "$build/policy-tests"
"$build/policy-tests"
for kind in Helper Fixture Cover; do
  bundle="$build/Chariox Computer $kind.app"
  if [[ $kind == Helper ]]; then bundle="$build/$helper_bundle"; fi
  mkdir -p "$bundle/Contents/MacOS"
  name=$(printf '%s' "$kind" | tr '[:upper:]' '[:lower:]')
  cat > "$bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>ai.chariox.computer-$name</string>
<key>CFBundleExecutable</key><string>$name</string>
<key>CFBundleName</key><string>Chariox Computer $kind</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
</dict></plist>
EOF
  if [[ $kind == Helper ]]; then /usr/libexec/PlistBuddy -c 'Add :LSUIElement bool true' "$bundle/Contents/Info.plist"; fi
  # M1 drill only: allowlist one kernel build for pairing, sealed by the helper signature.
  if [[ $kind == Helper && -n ${CHARIOX_DRILL_KERNEL_CDHASH:-} ]]; then
    /usr/libexec/PlistBuddy -c "Add :CharioxDrillKernelCDHash string $CHARIOX_DRILL_KERNEL_CDHASH" "$bundle/Contents/Info.plist"
  fi
  if [[ $kind == Helper ]]; then
    swiftc "${flags[@]}" Policy.swift Signing.swift Coordinates.swift PointerInput.swift WindowFence.swift Native.swift NativeSurface.swift TextClick.swift TextDrillReference.swift Pairing.swift PhysicalHuman.swift NativeInput.swift PairedNativeInput.swift PairedSession.swift CaptureDamage.swift CaptureAttachment.swift ProtectedPixels.swift NativeCaptureStream.swift CaptureProtection.swift PairedCapture.swift Helper.swift -o "$bundle/Contents/MacOS/helper"
  elif [[ $kind == Fixture ]]; then
    swiftc "${flags[@]}" Policy.swift Coordinates.swift Fixture.swift -o "$bundle/Contents/MacOS/fixture"
  else
    swiftc "${flags[@]}" Cover.swift -o "$bundle/Contents/MacOS/cover"
  fi
  codesign --force --sign - --identifier "ai.chariox.computer-$name" "$bundle"
  codesign --verify --strict "$bundle"
done
