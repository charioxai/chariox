# M5 owner signing dry run

These commands are prepared, not executed. The agent does not access Keychain,
use a signing key, submit to Apple, launch an installed bundle or change TCC.
Release remains BLOCKED. M4 protection, the matching grant, supported OS/upgrade
identity, real transports and full client/provider acceptance remain incomplete.
The codec does not change admission. Pairing still pins one exact kernel/helper
CDHash through the private drill contract, including Developer ID signed builds.
A stable production Team/identifier admission policy remains a release gate.

## Checklist before the one owner session

- Coordinator supplies a validated matching kernel under the permitted build root,
  its public unsigned SHA-256, version, and the approved source commit. b3 owns
  kernel tests. Do not rebuild the retained r6 bundle.
- Owner verifies the approved public Developer ID certificate fingerprint and
  Team `CM352DTZV6` in the protected inventory. Read the key-retention protocol
  and verify a protected backup with a tested restore before first key use.
  A local backup does not establish disaster recovery. No private key is exported.
- Owner supplies the existing protected notary profile name. No new credentials,
  certificate or profile are created by these commands. Do not print secrets.
- Choose a fresh build root for the unsigned helper and public fixtures. Preserve
  every signed artifact. Build the helper once with the final signed kernel's
  CDHash already in its Info.plist. Sign only those fresh unsigned artifacts.
- Confirm M4 Vault field mapping and private exclusions, native client integration,
  independent fixture receipts and the fatal press hook are ready. If they are
  absent, keep the owner suite BLOCKED. Signing alone does not clear acceptance.

Apple requires the hardened runtime for notarization and recommends stapling the
accepted ticket. See [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
and [custom workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).
No entitlement exception is required by this helper. Do not add get-task-allow,
JIT, unsigned memory or library-validation exceptions to force success.

## Exact commands for Miguel, conditional on readiness

Run personally from `cu/macos-m5`. Inputs below are public pins/paths/profile
names from the coordinator and protected owner inventory, not guessed defaults.
`CUMAC_KERNEL` must name a fresh unsigned matching kernel, never a signed one.
`CUMAC_SIGN_IDENTITY` is the approved public certificate SHA-1 fingerprint.
The coordinator must link the fresh kernel with `-no_adhoc_codesign`, for
example `RUSTFLAGS="-C link-arg=-no_adhoc_codesign"` in its approved build.
The unsigned helper build uses the same linker option. Do not remove a signature
from an existing kernel or helper to satisfy this prerequisite.

```bash
set -euo pipefail
: "${CUMAC_KERNEL:?fresh unsigned matching kernel under the approved build root}"
: "${CUMAC_UNSIGNED_KERNEL_SHA256:?approved public pre-sign pin}"
: "${CUMAC_SIGN_IDENTITY:?approved Developer ID certificate fingerprint}"
: "${CUMAC_NOTARY_PROFILE:?existing owner-managed profile name}"
: "${CUMAC_BUILD_DIR:?fresh owner build directory under the approved build root}"
: "${CUMAC_RELEASE_VERSION:?coordinator-approved version}"
case "$CUMAC_KERNEL" in /Users/miguel/.chariox/dev/cumac/build/*) ;; *) exit 1;; esac
case "$CUMAC_BUILD_DIR" in /Users/miguel/.chariox/dev/cumac/build/*) ;; *) exit 1;; esac
test ! -e "$CUMAC_BUILD_DIR"
test "$(shasum -a 256 "$CUMAC_KERNEL" | awk '{print $1}')" = "$CUMAC_UNSIGNED_KERNEL_SHA256"
# Preserve signed artifacts. This command must fail for an unsigned input.
if codesign -d "$CUMAC_KERNEL" 2>/dev/null; then exit 1; fi
codesign --sign "$CUMAC_SIGN_IDENTITY" --timestamp --options runtime \
  --identifier ai.chariox.kernel "$CUMAC_KERNEL"
codesign --verify --strict -R 'anchor apple generic and certificate leaf[subject.OU] = "CM352DTZV6"' "$CUMAC_KERNEL"
CHARIOX_DRILL_KERNEL_CDHASH=$(codesign -dvv "$CUMAC_KERNEL" 2>&1 | sed -n 's/^CDHash=//p')
test ${#CHARIOX_DRILL_KERNEL_CDHASH} -eq 40
export CHARIOX_DRILL_KERNEL_CDHASH CUMAC_BUILD_DIR
CUMAC_UNSIGNED=1 bash apps/macos-computer-use/build.sh
for kind in Helper Fixture Cover; do
  cu_bundle="$CUMAC_BUILD_DIR/Chariox Computer $kind.app"
  if codesign -d "$cu_bundle" 2>/dev/null; then exit 1; fi
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $CUMAC_RELEASE_VERSION" "$cu_bundle/Contents/Info.plist"
  codesign --sign "$CUMAC_SIGN_IDENTITY" --timestamp --options runtime "$cu_bundle"
  codesign --verify --strict -R 'anchor apple generic and certificate leaf[subject.OU] = "CM352DTZV6"' "$cu_bundle"
done
cu_archive="$CUMAC_BUILD_DIR/notary-submit.zip"
cu_payload="$CUMAC_BUILD_DIR/notary-payload"
mkdir "$cu_payload"
for kind in Helper Fixture Cover; do
  ditto "$CUMAC_BUILD_DIR/Chariox Computer $kind.app" "$cu_payload/Chariox Computer $kind.app"
done
ditto "$CUMAC_KERNEL" "$cu_payload/chariox-kernel"
ditto -c -k --keepParent "$cu_payload" "$cu_archive"
xcrun notarytool submit "$cu_archive" --keychain-profile "$CUMAC_NOTARY_PROFILE" \
  --wait --output-format json > "$CUMAC_BUILD_DIR/notary-result.json"
python3 - "$CUMAC_BUILD_DIR/notary-result.json" <<'PY'
import json, sys
assert json.load(open(sys.argv[1]))['status'] == 'Accepted', 'notarization not accepted'
PY
for kind in Helper Fixture Cover; do
  cu_bundle="$CUMAC_BUILD_DIR/Chariox Computer $kind.app"
  xcrun stapler staple "$cu_bundle"
  xcrun stapler validate "$cu_bundle"
  codesign --verify --strict "$cu_bundle"
  spctl --assess --type execute "$cu_bundle"
done
codesign --verify --strict "$CUMAC_KERNEL"
# Bare Mach-O cannot be stapled. Its ticket is online; offline distribution
# needs a notarized/stapled package or disk image, still a release blocker.
for kind in Helper Fixture Cover; do
  cu_bundle="$CUMAC_BUILD_DIR/Chariox Computer $kind.app"
  codesign -dvv "$cu_bundle" 2> "$CUMAC_BUILD_DIR/$kind-signature.txt"
  shasum -a 256 "$cu_bundle/Contents/MacOS/$(printf '%s' "$kind" | tr '[:upper:]' '[:lower:]')" > "$CUMAC_BUILD_DIR/$kind-sha256.txt"
done
codesign -dvv "$CUMAC_KERNEL" 2> "$CUMAC_BUILD_DIR/kernel-signature.txt"
shasum -a 256 "$CUMAC_KERNEL" > "$CUMAC_BUILD_DIR/kernel-sha256.txt"
```

Do not run an identity listing, Keychain unlock, certificate import or credential
bootstrap as a dry run. A safe agent dry run is `bash -n build.sh`, source and
plist inspection, and checking that the prepared commands use fresh paths and
fail closed. The agent ran only that level of checking.

After Accepted, rearchive the stapled bundles for distribution, publish the
public executable hashes/CDHashes and notarization receipt, then grant the exact
helper in the single owner session. Do not change its code, Info.plist, signature
or kernel allowlist after the grant. Upgrade testing must prove unchanged
Team/identifier requirements, reject foreign signatures, retire old refs and
seats, preserve durable keys and recover owned input. No install/upgrade or
production release is claimed by this source checkpoint.
