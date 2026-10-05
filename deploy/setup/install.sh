#!/bin/sh
# MP-07 / MP-08 / MP-11: versioned public bootstrap; only public trust inputs embedded.
set -eu
umask 077
version='@VERSION@'
release_key='@RELEASE_PUBLIC_KEY@'
base='@RELEASE_BASE@'
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) platform=linux-x64 ;;
  Darwin:arm64) platform=darwin-arm64 ;;
  *) printf '%s\n' 'MP-07/MP-08/MP-11: unsupported Setup platform' >&2; exit 1 ;;
esac
# No argument may carry a code. --enroll takes no value; Setup reads stdin or prompts once.
previous=
for argument do
  if [ "$previous" = --enroll ]; then
    case "$argument" in --*) ;; *) printf '%s\n' 'MP-11: enrollment codes must be supplied on stdin' >&2; exit 1 ;; esac
  fi
  previous=$argument
done
case "$version" in ''|*@*) printf '%s\n' 'MP-07: use a published versioned installer' >&2; exit 1 ;; esac
stage=$(mktemp -d "${TMPDIR:-/tmp}/chariox-setup.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
curl -fsS --location --max-redirs 5 --proto-redir '=https' --proto '=https' --max-time 300 --max-filesize 268435456 "$base/v$version/chariox-setup-$version-$platform" -o "$stage/chariox-setup"
curl -fsS --location --max-redirs 5 --proto-redir '=https' --proto '=https' --max-time 30 --max-filesize 128 "$base/v$version/chariox-setup-$version-$platform.sig" -o "$stage/signature.hex"
python3 - "$release_key" "$stage" <<'PY'
import base64, pathlib, sys
key, stage = sys.argv[1], pathlib.Path(sys.argv[2])
if len(key) != 64 or any(c not in '0123456789abcdef' for c in key): sys.exit('MP-07: invalid published release pin')
sig = (stage / 'signature.hex').read_text()
if len(sig) != 128 or any(c not in '0123456789abcdef' for c in sig): sys.exit('MP-07: invalid Setup signature')
(stage / 'signature').write_bytes(bytes.fromhex(sig))
spki = bytes.fromhex('302a300506032b6570032100' + key)
(stage / 'public.pem').write_text('-----BEGIN PUBLIC KEY-----\n' + base64.b64encode(spki).decode() + '\n-----END PUBLIC KEY-----\n')
PY
if ! openssl pkeyutl -verify -pubin -inkey "$stage/public.pem" -rawin -in "$stage/chariox-setup" -sigfile "$stage/signature" >/dev/null 2>&1; then
  printf '%s\n' 'MP-07/MP-11: Setup signature refused' >&2; exit 1
fi
chmod 700 "$stage/chariox-setup"
"$stage/chariox-setup" --install-only --login "$@"
