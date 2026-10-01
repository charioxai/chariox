#!/usr/bin/env bash
# Installs a Chariox Linux release bundle: the one root step, after checking the bundle.
#
#   sudo ./install.sh --user NAME --release-key HEX [--dry-run]
#   ./install.sh --check --release-key HEX
#
# --release-key is the release's Ed25519 public key as published by Chariox (64
# hexadecimal characters), never read from this directory.
#
# 1. Copies the bundle into a private root-owned directory and checks that copy:
#    manifest.sig against --release-key, then every file's size and SHA-256,
#    that no file was added, and the App runtime's inventory signature against
#    the runtime key the signed manifest names. --check stops here and needs no root.
# 2. Runs deploy/local-linux/install-root.sh for --user with the App runtime key
#    and inventory digest from the signed manifest.
# 3. Installs the slice build context the kernel provisions slices from into
#    /usr/lib/chariox/slice-build-context, and chariox (the CLI/TUI) and
#    chariox-app-package into /usr/local/bin.
# 4. Prints the ordinary user's step, deploy/local-linux/install-user.sh, which
#    installs and starts the kernel as that user's systemd --user unit.
#
# Requires bash, python3, openssl, diffutils and coreutils. To remove Chariox, see
# deploy/local-linux/install-user.sh uninstall and install-root.sh uninstall, then
# remove /usr/lib/chariox/slice-build-context and the two /usr/local/bin commands.
set -euo pipefail
umask 022

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
say() { printf '[chariox-install] %s\n' "$*"; }
die() { printf '[chariox-install] error: %s\n' "$*" >&2; exit 1; }
usage() { echo "usage: sudo ./install.sh --user NAME --release-key HEX [--dry-run] | ./install.sh --check --release-key HEX"; }

users=() key= check=0 dry_run=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) users+=("${2:?--user needs a name}"); shift 2 ;;
    --release-key) key=${2:?--release-key needs a key}; shift 2 ;;
    --check) check=1; shift ;;
    --dry-run) dry_run=(--dry-run); shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done
[[ "$key" =~ ^[0-9a-f]{64}$ ]] || { usage >&2; die "--release-key must be 64 lowercase hexadecimal characters"; }
(( check )) || [[ ${#users[@]} -gt 0 ]] || { usage >&2; exit 2; }
(( check )) || [[ "$(id -u)" == 0 ]] || die "run this as root (sudo), or use --check"

# check DIR: verifies DIR against the signed manifest; prints "KEY DIGEST VERSION".
check_bundle() {
  python3 - "$1" "$key" <<'PY'
import base64, hashlib, json, os, subprocess, sys, tempfile
root, key = sys.argv[1], bytes.fromhex(sys.argv[2])
def fail(message): sys.exit(f"[chariox-install] error: {message}")
def verify(key, message, signature, what):
    if len(signature) != 128 or any(c not in "0123456789abcdef" for c in signature):
        fail(f"{what} must be 128 lowercase hexadecimal characters")
    spki = bytes.fromhex("302a300506032b6570032100") + key
    pem = "-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(spki).decode() + "\n-----END PUBLIC KEY-----\n"
    with tempfile.TemporaryDirectory() as scratch:
        paths = {name: os.path.join(scratch, name) for name in ("key.pem", "signature", "message")}
        for name, data in (("key.pem", pem.encode()), ("signature", bytes.fromhex(signature)), ("message", message)):
            open(paths[name], "wb").write(data)
        verified = subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", paths["key.pem"], "-rawin",
                                   "-in", paths["message"], "-sigfile", paths["signature"]], capture_output=True)
    return verified.returncode == 0
manifest_bytes = open(os.path.join(root, "manifest.json"), "rb").read()
signature = open(os.path.join(root, "manifest.sig"), "rb").read().decode("ascii", "replace")
if not verify(key, manifest_bytes, signature, "manifest.sig"):
    fail("manifest.sig does not verify with the release key: this bundle is not that release")
manifest = json.loads(manifest_bytes)
if manifest.get("schema") != "chariox.release-bundle.v1" or manifest.get("platform") != "linux-x64":
    fail("this is not a Chariox linux-x64 release bundle")
listed = {entry["path"]: entry for entry in manifest["files"]}
for directory, subdirectories, names in os.walk(root):
    for name in subdirectories + names:
        path = os.path.join(directory, name)
        if os.path.islink(path) or not (os.path.isdir(path) or os.path.isfile(path)):
            fail(f"{os.path.relpath(path, root)} is a link or special file")
    for name in names:
        relative = os.path.relpath(os.path.join(directory, name), root)
        if relative in ("manifest.json", "manifest.sig"):
            continue
        entry = listed.pop(relative, None)
        if entry is None:
            fail(f"{relative} is not in the signed manifest")
        data = open(os.path.join(root, relative), "rb").read()
        if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
            fail(f"{relative} does not match the signed manifest")
if listed:
    fail("the bundle is missing " + ", ".join(sorted(listed)))
# The App runtime's own inventory signature, as root enrollment will check it.
runtime = manifest["runtime"]
inventory = open(os.path.join(root, "runtime", "runtime-inventory.json"), "rb").read()
if hashlib.sha256(inventory).hexdigest() != runtime["inventorySha256"]:
    fail("runtime/runtime-inventory.json is not the inventory the signed manifest names")
inventory_signature = open(os.path.join(root, "runtime", "runtime-inventory.sig"), "rb").read().decode("ascii", "replace")
if not verify(bytes.fromhex(runtime["publicKeyHex"]), inventory, inventory_signature, "runtime/runtime-inventory.sig"):
    fail("runtime/runtime-inventory.sig does not verify with the runtime key in the signed manifest")
print(runtime["publicKeyHex"], runtime["inventorySha256"], manifest["version"])
PY
}

if (( check )); then
  read -r _ _ version < <(check_bundle "$here") || exit 1
  say "bundle $here is Chariox $version, signed by the release key"
  exit 0
fi

stage=$(mktemp -d /var/tmp/chariox-install.XXXXXXXX)
trap 'rm -rf -- "$stage"' EXIT
chmod 0700 "$stage"
# Check and install from a root-owned copy, so the source directory cannot change underneath.
cp -R -- "$here/." "$stage/"
chown -R 0:0 "$stage"
chmod -R go-w "$stage"
read -r runtime_key runtime_digest version < <(check_bundle "$stage") || exit 1
say "bundle is Chariox $version, signed by the release key"

args=(install)
for user in "${users[@]}"; do args+=(--user "$user"); done
"$stage/deploy/local-linux/install-root.sh" "${args[@]}" --bin "$stage/libexec" --runtime "$stage/runtime" \
  --runtime-key "$runtime_key" --runtime-digest "$runtime_digest" "${dry_run[@]}"

# A release kernel runs its slice provisioner from the system-wide slice build context.
context=/usr/lib/chariox/slice-build-context
if [[ ${#dry_run[@]} -gt 0 ]]; then
  say "would install $context"
elif [[ -d "$context" && ! -L "$context" ]] && diff -rq -- "$stage/share/chariox/slice-build-context" "$context" > /dev/null 2>&1; then
  say "unchanged $context"
else
  say "install $context"
  install -d -m 0755 -o 0 -g 0 /usr/lib/chariox
  rm -rf -- "$context.chariox-new" "$context.chariox-old"
  cp -R -- "$stage/share/chariox/slice-build-context" "$context.chariox-new"
  if [[ -e "$context" || -L "$context" ]]; then mv -- "$context" "$context.chariox-old"; fi
  mv -- "$context.chariox-new" "$context"
  rm -rf -- "$context.chariox-old"
fi

for name in chariox chariox-app-package; do
  if [[ ${#dry_run[@]} -gt 0 ]]; then
    say "would install /usr/local/bin/$name"
  elif cmp -s -- "$stage/bin/$name" "/usr/local/bin/$name"; then
    say "unchanged /usr/local/bin/$name"
  else
    say "install /usr/local/bin/$name"
    install -D -m 0755 -o 0 -g 0 -- "$stage/bin/$name" "/usr/local/bin/$name.chariox-new"
    mv -f -- "/usr/local/bin/$name.chariox-new" "/usr/local/bin/$name"
  fi
done

for user in "${users[@]}"; do
  say "next, as $user (no root): $here/deploy/local-linux/install-user.sh install --kernel $here/bin/chariox-kernel"
done
