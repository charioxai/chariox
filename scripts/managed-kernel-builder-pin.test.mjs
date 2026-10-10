import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFile, mkdtemp, mkdir, writeFile, rm, chmod } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test as nodeTest } from "node:test"

// MP-11: qualify actual root ownership, matching the recovery drill harness.
const test = (name, run) => nodeTest(name, {
  skip: process.platform !== "linux" || process.getuid() !== 0,
}, run)

const helper = new URL("../deploy/managed-kernel/managed-kernel-builder-pin-transaction.sh", import.meta.url).pathname
const upgrade = await readFile(new URL("../deploy/managed-kernel/upgrade-image.sh", import.meta.url), "utf8")
const checks = ["path_exists", "require_regular_file", "require_directory", "require_root_owned_directory", "require_private_regular_file", "require_root_owned_private_regular_file", "require_root_owned_ancestor_chain", "read_single_line", "validate_digest"]
  .map(name => { const fn = upgrade.match(new RegExp(`^${name}\\(\\) \\{\\n[\\s\\S]*?^\\}`, "m"))?.[0]; assert.ok(fn, name); return fn }).join("\n")

// MP-07/MP-11: exercise the real shell checks and atomic writer, not a model
// of their behavior. All key-shaped files contain only synthetic public text.
for (const failure of [null, "unsafe-journal", "missing-previous", "invalid-target-digest", "different-target-pin", "unsafe-release", "unsafe-runtime", "unrelated-runtime", "atomic-write"]) test(`MP-07 builder pin ${failure ?? "activation"} retains caller settlement and public substeps`, async context => {
  const root = await mkdtemp(join(tmpdir(), "path1-pin-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const journal = join(root, "journal"), releases = join(root, "releases"), runtime = join(root, "runtime-pin")
  await mkdir(journal)
  for (const [role, hex] of [["previous", "a"], ["target", "b"]]) {
    const keyDir = join(releases, hex.repeat(64), "usr/lib/chariox")
    await mkdir(keyDir, { recursive: true })
    await writeFile(join(journal, `${role}-digest`), `sha256:${hex.repeat(64)}\n`, { mode: 0o600 })
    await writeFile(join(journal, `${role}-builder-public-key`), `public-${role}\n`, { mode: 0o600 })
    await writeFile(join(keyDir, "builder-public-key"), `public-${role}\n`, { mode: 0o644 })
  }
  await writeFile(runtime, failure === "unrelated-runtime" ? "public-unrelated\n" : "public-previous\n", { mode: 0o644 })
  if (failure === "unsafe-journal") await chmod(join(journal, "target-builder-public-key"), 0o666)
  if (failure === "missing-previous") await rm(join(journal, "previous-builder-public-key"))
  if (failure === "invalid-target-digest") await writeFile(join(journal, "target-digest"), "invalid\n")
  if (failure === "different-target-pin") await writeFile(join(journal, "target-builder-public-key"), "public-unrelated\n")
  if (failure === "unsafe-release") await chmod(join(releases, "b".repeat(64), "usr/lib/chariox/builder-public-key"), 0o666)
  if (failure === "unsafe-runtime") await chmod(runtime, 0o666)
  const scriptRoot = failure === "atomic-write" ? join(root, "missing-writer") : new URL("../deploy/managed-kernel", import.meta.url).pathname
  const command = `${checks}\n. "$1"\nmanaged_provider_topology=path1\nreleases_root=$3\ntrusted_builder_runtime_key=$4\nscript_root=$5\nrecord_diagnostic_phase() { printf '%s\\n' "$1"; }\nif activate_builder_pin "$2" target; then printf 'activated\\n'; else printf 'rollback-reached\\n'; fi\n`
  const result = spawnSync("sh", ["-c", command, "test", helper, journal, releases, runtime, scriptRoot], { encoding: "utf8", timeout: 10_000 })
  assert.equal(result.status, 0, result.stderr)
  if (failure) {
    assert.match(result.stdout, /rollback-reached/)
    assert.match(result.stdout, /builder_pin_failed/)
    const categories = {
      "unsafe-journal": "builder_pin_journal_target_file_failed",
      "missing-previous": "builder_pin_journal_previous_missing",
      "invalid-target-digest": "builder_pin_journal_target_digest_failed",
      "different-target-pin": "builder_pin_journal_target_compare_failed",
      "unsafe-release": "builder_pin_journal_target_release_file_failed",
      "unsafe-runtime": "builder_pin_runtime_file_failed",
      "unrelated-runtime": "builder_pin_compare_failed",
      "atomic-write": "builder_pin_atomic_failed",
    }
    assert.ok(result.stdout.trim().split("\n").includes(categories[failure]), result.stdout)
    assert.equal(await readFile(runtime, "utf8"), failure === "unrelated-runtime" ? "public-unrelated\n" : "public-previous\n")
  } else {
    assert.equal(await readFile(runtime, "utf8"), "public-target\n")
    assert.deepEqual(result.stdout.trim().split("\n"), ["builder_pin_journal_start", "builder_pin_journal_returned", "builder_pin_runtime_start", "builder_pin_runtime_returned", "builder_pin_compare_start", "builder_pin_compare_returned", "builder_pin_atomic_start", "builder_pin_atomic_returned", "activated"])
  }
})
