import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmod, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const state = new URL("./managed-kernel-upgrade-state.mjs", import.meta.url).pathname
const upgrade = await readFile(new URL("./upgrade-image.sh", import.meta.url), "utf8")
const id = `managed_release_update_${"a".repeat(36)}`
const from = `sha256:${"b".repeat(64)}`
const target = `sha256:${"c".repeat(64)}`

test("MP-07 recovery binds the root journal to the original Cloud update", { skip: process.getuid() !== 0 }, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-mp07-recovery-"))
  try {
    const identity = join(root, "update-result-identity")
    const writeIdentity = values => writeFile(identity, ["1", ...values, "environment", "machine", "kernel", ""].join("\n"), { mode: 0o600 })
    const validate = () => spawnSync(process.execPath, [state, "validate-update-recovery", root, id, from, target], { encoding: "utf8", timeout: 5000 })
    await writeIdentity([id, from, target])
    assert.equal(validate().status, 0)
    for (const values of [[id.replace(/a$/, "d"), from, target], [id, target, target], [id, from, from]]) {
      await writeIdentity(values)
      assert.match(validate().stderr, /does not match the Cloud command/)
    }
    await writeIdentity([id, from, target])
    await chmod(identity, 0o666)
    assert.match(validate().stderr, /identity is unsafe/)
    await chmod(identity, 0o600)
    await writeFile(identity, "x".repeat(4097))
    assert.match(validate().stderr, /identity is unsafe/)
    await rm(identity)
    await symlink("/missing", identity)
    assert.notEqual(validate().status, 0)
  } finally { await rm(root, { recursive: true, force: true }) }
})

test("MP-07 recovery-only settles before target activation and does not need an image", () => {
  const imageGuard = upgrade.match(/if \[ "\$recover_only" -eq 0 \] && \{ \[ -L "\$image_root" \] \|\| \[ ! -d "\$image_root" \]; \}; then[\s\S]*?\nfi/)[0]
  const recovery = upgrade.slice(upgrade.indexOf("\nrecover_transaction\n") + 1, upgrade.indexOf("\nselect_receipt_path", upgrade.indexOf("\nrecover_transaction\n")))
  for (const status of [0, 1]) {
    const result = spawnSync("sh", ["-c", `set -eu\nrecover_only=1\nimage_root=/nonexistent-mp07-image\n${imageGuard}\nrecover_transaction() { return ${status}; }\n${recovery}\nexit 42\n`], { encoding: "utf8", timeout: 5000 })
    assert.equal(result.status, status, result.stderr)
  }
})
