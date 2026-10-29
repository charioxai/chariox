#!/bin/sh
set -eu
umask 077

SOURCE_REPOSITORY=${1:-}
REVISION=${2:-}
OUTPUT=${3:-}

SCRIPT_DIRECTORY=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

fail() {
  echo "Chariox staging release creation failed: $*" >&2
  exit 1
}

[ "$(git -C "$SOURCE_REPOSITORY" rev-parse --is-inside-work-tree 2>/dev/null || true)" = true ] \
  || fail "argument 1 must be a Chariox Cloud Git worktree"
case "$REVISION" in
  *[!0-9a-f]*|'') fail "argument 2 must be the exact 40-character Git revision" ;;
esac
[ "${#REVISION}" -eq 40 ] || fail "argument 2 must be the exact 40-character Git revision"
[ "$(git -C "$SOURCE_REPOSITORY" rev-parse "${REVISION}^{commit}" 2>/dev/null || true)" = "$REVISION" ] \
  || fail "the exact revision is not available in the source repository"
[ -n "$OUTPUT" ] || fail "argument 3 must be a new release archive path"
[ ! -e "$OUTPUT" ] && [ ! -L "$OUTPUT" ] || fail "release archive output already exists"
OUTPUT_DIRECTORY=$(dirname "$OUTPUT")
[ -d "$OUTPUT_DIRECTORY" ] || fail "release archive output directory does not exist"

PYTHON_BIN=$(command -v python3 2>/dev/null || true)
case "$PYTHON_BIN" in
  /*) ;;
  *) fail "python3 is not an absolute executable" ;;
esac
REALPATH_BIN=$(command -v realpath 2>/dev/null || true)
case "$REALPATH_BIN" in
  /*) [ -x "$REALPATH_BIN" ] || fail "realpath is required to resolve python3" ;;
  *) fail "realpath is required to resolve python3" ;;
esac
PYTHON_BIN=$("$REALPATH_BIN" -- "$PYTHON_BIN" 2>/dev/null || true)
[ -f "$PYTHON_BIN" ] && [ -x "$PYTHON_BIN" ] || fail "python3 is not executable"

WORK_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/chariox-staging-release.XXXXXX")
case "$WORK_ROOT" in
  "${TMPDIR:-/tmp}"/chariox-staging-release.*) ;;
  *) fail "unsafe temporary release directory" ;;
esac
ARCHIVE_TMP=$OUTPUT.tmp.$$
cleanup() {
  rm -rf -- "$WORK_ROOT"
  rm -f -- "$ARCHIVE_TMP"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

RELEASE_ROOT=$WORK_ROOT/release
install -d -m 0700 "$RELEASE_ROOT"
SOURCE_ARCHIVE=$WORK_ROOT/source.tar
git -C "$SOURCE_REPOSITORY" archive --format=tar "$REVISION" > "$SOURCE_ARCHIVE" \
  || fail "could not archive the exact Git revision"
"$PYTHON_BIN" "$SCRIPT_DIRECTORY/extract-cloud-staging-release.py" \
  "$SOURCE_ARCHIVE" "$RELEASE_ROOT" \
  || fail "source archive extraction failed"
# The private creator umask must not leak into the immutable release. The
# publication-services validator runs as a non-root user and needs to traverse
# and read the reviewed Git tree after it is mounted read-only.
chmod -R a+rX "$RELEASE_ROOT"
SOURCE_SHA256=$(node "$SCRIPT_DIRECTORY/release-source-digest.mjs" "$RELEASE_ROOT")
case "$SOURCE_SHA256" in
  *[!0-9a-f]*|'') fail "release source digest is invalid" ;;
esac
[ "${#SOURCE_SHA256}" -eq 64 ] || fail "release source digest is invalid"
printf '%s\n' "$REVISION" > "$RELEASE_ROOT/.chariox-release-revision"
printf '%s\n' "$SOURCE_SHA256" > "$RELEASE_ROOT/.chariox-release-source-sha256"
chmod 0644 \
  "$RELEASE_ROOT/.chariox-release-revision" \
  "$RELEASE_ROOT/.chariox-release-source-sha256"
COPYFILE_DISABLE=1 tar --no-xattrs -czf "$ARCHIVE_TMP" -C "$RELEASE_ROOT" .
if tar -tzf "$ARCHIVE_TMP" | grep -Eq '(^|/)\._'; then
  fail "release archive contains macOS AppleDouble metadata"
fi
chmod 0600 "$ARCHIVE_TMP"
mv "$ARCHIVE_TMP" "$OUTPUT"

printf 'release_revision=%s\n' "$REVISION"
printf 'release_source_sha256=%s\n' "$SOURCE_SHA256"
