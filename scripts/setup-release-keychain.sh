#!/bin/bash
set -euo pipefail
node scripts/release-keychain-state.mjs snapshot "$RUNNER_TEMP"
keychain="$RUNNER_TEMP/release.keychain-db"
setup_complete=false
cleanup_setup() {
  status=$?
  rm -f "$RUNNER_TEMP/identity.p12" "$RUNNER_TEMP/notary.p8"
  if [[ "$setup_complete" != true ]]; then
    node scripts/release-keychain-state.mjs cleanup "$RUNNER_TEMP" || status=1
  fi
  exit "$status"
}
trap cleanup_setup EXIT
echo "RELEASE_KEYCHAIN=$keychain" >> "$GITHUB_ENV"
password=$(uuidgen)
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
(umask 077; printf '%s' "$P12" | base64 --decode > "$RUNNER_TEMP/identity.p12"; printf '%s' "$NOTARY_KEY" > "$RUNNER_TEMP/notary.p8")
security import "$RUNNER_TEMP/identity.p12" -k "$keychain" -P "$P12_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain"
security default-keychain -d user -s "$keychain"
xcrun notarytool store-credentials "$CHARIOX_NOTARY_PROFILE" --key "$RUNNER_TEMP/notary.p8" \
  --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" --keychain "$keychain"
setup_complete=true
