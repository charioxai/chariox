import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdtemp, mkdir, readFile, readlink, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = new URL("./provision-linux-docker-slice.sh", import.meta.url)
function section(source, start, end) {
  const first = source.indexOf(start), last = source.indexOf(end, first + start.length)
  assert.ok(first >= 0 && last > first, "production restore functions must be present")
  return source.slice(first, last)
}

async function fixture(t, managed, failExtract) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-restore-stream-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const sourceRoot = join(root, "source"), destination = join(root, "home")
  await Promise.all([mkdir(sourceRoot), mkdir(destination)])
  await writeFile(join(sourceRoot, "public-state"), "synthetic saved home bytes", { mode: 0o600 })
  await symlink("public-state", join(sourceRoot, "state-link"))
  const archive = join(root, "saved.tar")
  const tar = spawnSync("tar", ["-cf", archive, "-C", sourceRoot, "."], { encoding: "utf8" })
  assert.equal(tar.status, 0, tar.stderr)
  const production = await readFile(provisioner, "utf8")
  // Execute the actual emitted production function and its timeout helpers.
  // Docker is a transport stub; it extracts a real tiny plain tar from the
  // received stdin. Real zstd/Docker integration is checked separately.
  const dockerScript = ["#!/usr/bin/env bash", "set -Eeuo pipefail",
    "docker() {",
    "  printf '%s\\n' \"$*\" >> \"$CALLS\"",
    "  case \"$1\" in",
    "    rm|create|start) return 0 ;;",
    "    cp) : > \"$LAYER_ARCHIVE\"; return 93 ;;",
    "    exec)",
    "      shift; local attached=0",
    "      if [[ \"$1\" == -i ]]; then attached=1; shift; fi",
    "      [[ \"$1\" == -u && \"$2\" == root ]] || return 90; shift 3",
    "      [[ \"$1\" == /bin/sh && \"$2\" == -c ]] || return 94",
    "      [[ \"$FAIL_EXTRACT\" == 0 ]] || return 42",
    "      if (( attached )); then tee \"$RECEIVED\" | tar -xf - -C \"$DESTINATION\"",
    "      else tar -xf \"$ARCHIVE\" -C \"$DESTINATION\"; fi ;;",
    "    *) return 90 ;;",
    "  esac",
    "}",
    'docker "$@"',
  ].join("\n")
  await writeFile(join(root, "docker"), dockerScript, { mode: 0o755 })
  const script = [
    "set -Eeuo pipefail", "log() { :; }", "fail() { return 91; }",
    section(production, "run_guarded_command() {", "usage() {"),
    section(production, "restore_saved_home_volume() {", "prepare_home_volume() {"),
    "SLICE_SAVED_HOME_ARCHIVE=\"$ARCHIVE\"", "SLICE_SAVED_HOME_ARCHIVE_DIR=\"$ARCHIVE_DIR\"",
    "SLICE_NAME=synthetic-slice", "SLICE_HOME_VOLUME=synthetic-home", "SLICE_IMAGE=synthetic-image",
    "status=0; restore_saved_home_volume || status=$?; printf 'STATUS=%s\\n' \"$status\"",
  ].join("\n")
  const result = spawnSync("bash", ["-c", script], {
    encoding: "utf8", timeout: 20_000,
    env: { PATH: `${root}:${process.env.PATH}`, TMPDIR: root, SCRIPT_DIR: fileURLToPath(new URL(".", import.meta.url)), DESTINATION: destination, ARCHIVE: archive,
      ARCHIVE_DIR: managed ? root : "", CALLS: join(root, "calls"), LAYER_ARCHIVE: join(root, "layer-archive"),
      RECEIVED: join(root, "received"), FAIL_EXTRACT: failExtract ? "1" : "0" },
  })
  assert.equal(result.status, 0, result.stderr || String(result.error))
  return { root, destination, archive, result, calls: (await readFile(join(root, "calls"), "utf8")).trim().split("\n") }
}

for (const managed of [false, true]) test("saved-home restore uses exact stream or pinned mount (managed=" + managed + ")", async t => {
  const f = await fixture(t, managed, false)
  assert.match(f.result.stdout, /STATUS=0/)
  assert.equal(await readFile(join(f.destination, "public-state"), "utf8"), "synthetic saved home bytes")
  assert.equal((await stat(join(f.destination, "public-state"))).mode & 0o777, 0o600)
  assert.equal(await readlink(join(f.destination, "state-link")), "public-state")
  assert.equal(await readFile(join(f.destination, "state-link"), "utf8"), "synthetic saved home bytes")
  await assert.rejects(stat(join(f.root, "layer-archive")), { code: "ENOENT" })
  assert.equal(f.calls.some(call => call.startsWith("cp ")), false)
  assert.match(f.calls.at(-1), /^rm -f synthetic-slice-home-restore-\d+$/)
  if (managed) {
    assert.ok(f.calls.some(call => call.includes(f.root + ":/restore:ro")))
    await assert.rejects(stat(join(f.root, "received")), { code: "ENOENT" })
  } else {
    const original = await readFile(f.archive), received = await readFile(join(f.root, "received"))
    assert.equal(received.length, original.length)
    assert.equal(createHash("sha256").update(received).digest("hex"), createHash("sha256").update(original).digest("hex"))
  }
})

for (const managed of [false, true]) test("failed extraction removes helper without layer archive (managed=" + managed + ")", async t => {
  const f = await fixture(t, managed, true)
  assert.match(f.result.stdout, /STATUS=42/)
  assert.match(f.calls.at(-1), /^rm -f synthetic-slice-home-restore-\d+$/)
  assert.equal(f.calls.some(call => call.startsWith("cp ")), false)
  await assert.rejects(stat(join(f.root, "layer-archive")), { code: "ENOENT" })
})
