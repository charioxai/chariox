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

test("MP-07 the root updater recovers independently when the kernel is unavailable", async () => {
  const runtime = await readFile(new URL("../../apps/kernel/src/runtime/managed_release_update.rs", import.meta.url), "utf8")
  const launcher = runtime.slice(runtime.indexOf("fn start_update_unit("), runtime.indexOf("fn start_update_unit(") + 1800)
  for (const property of ["Restart=on-failure", "RestartSec=2s", "StartLimitIntervalSec=60s", "StartLimitBurst=5"]) {
    assert.ok(launcher.includes(`--property=${property}`), `missing independent updater recovery: ${property}`)
  }
})

// Supplementary real-systemd contract drill; paid A/B recovery remains required.
test("MP-07 killed root updater restarts without a kernel", { skip: process.env.CHARIOX_MP07_SYSTEMD_DRILL !== "1" }, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-mp07-unit-"))
  const unit = `chariox-path1-r39-recovery-${process.pid}`
  const call = (bin, args) => spawnSync(bin, args, { encoding: "utf8", timeout: 10_000 })
  const runtime = await readFile(new URL("../../apps/kernel/src/runtime/managed_release_update.rs", import.meta.url), "utf8")
  const launcher = runtime.slice(runtime.indexOf("fn start_update_unit("), runtime.indexOf("fn start_update_unit(") + 1800)
  const properties = [...launcher.matchAll(/"(--property=[^"\n]+)"/g)].map(match => match[1])
  const script = join(root, "run.sh")
  await writeFile(script, `#!/bin/sh
set -eu
if test -f '${root}/pending'; then touch '${root}/recovered'; exit 0; fi
touch '${root}/pending'
exec /bin/sleep 60
`, { mode: 0o700 })
  try {
    const started = call("systemd-run", [`--unit=${unit}`, "--collect", "--quiet", ...properties, script])
    assert.equal(started.status, 0, started.stderr)
    const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
    for (let n = 0; n < 30; n++) { try { await readFile(join(root, "pending")); break } catch { await sleep(100) } }
    await readFile(join(root, "pending"))
    const pid = Number(call("systemctl", ["show", "--value", "--property=MainPID", unit]).stdout.trim())
    // Reject every unsafe signal target and verify this run owns the exact cgroup.
    assert.ok(Number.isSafeInteger(pid) && pid > 1)
    assert.ok((await readFile(`/proc/${pid}/cgroup`, "utf8")).includes(`/${unit}.service`))
    process.kill(pid, "SIGKILL")
    let recovered = false
    for (let n = 0; n < 60; n++) { try { await readFile(join(root, "recovered")); recovered = true; break } catch { await sleep(100) } }
    assert.equal(recovered, true, "root transaction remains stranded while the kernel is unavailable")
  } finally {
    call("systemctl", ["stop", unit])
    call("systemctl", ["reset-failed", unit])
    await rm(root, { recursive: true, force: true })
  }
})
