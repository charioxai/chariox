// MP-07 / MP-08 / MP-11: real localhost SSH; synthetic signed bytes and mocked user-service manager.
import assert from "node:assert/strict"
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises"
import { join } from "node:path"
import test from "node:test"
import { createFixture } from "./fixture.mjs"
import { runSshMachine } from "./transport.mjs"

import { sshTarget, availablePort } from "./ssh-target-fixture.mjs"
test("MP-07/MP-08/MP-11 actual SSH install/repeat/two IDs/stop/remove/tamper and cleanup", async t => {
  const { scratch, home } = await sshTarget(t)
  const f = await createFixture(scratch)
  const release = { archive: f.archive, releasePublicKey: f.releasePublicKey, builderPublicKey: f.builderPublicKey, releasePublicKeyFingerprint: f.releasePublicKeyFingerprint, builderPublicKeyFingerprint: f.builderPublicKeyFingerprint }
  const baseRequest = { action: "install", installId: "byom-one", port: await availablePort(), releaseDigest: f.releaseDigest }
  const staging = join(home, ".chariox/dev/md-staging")
  await mkdir(staging, { recursive: true }); await writeFile(join(staging, "sentinel"), "keep")
  const one = await runSshMachine("byom-local", baseRequest, release)
  assert.deepEqual(one, { installId: "byom-one", status: "installed", releaseDigest: f.releaseDigest, enrolled: false })
  assert.deepEqual(await runSshMachine("byom-local", baseRequest, release), one)
  const twoRequest = { ...baseRequest, installId: "byom-two", port: await availablePort() }
  assert.equal((await runSshMachine("byom-local", twoRequest, release)).status, "installed")
  // Authentication keys are synthetic and never copied into either install.
  assert.equal((await runSshMachine("byom-local", { ...baseRequest, action: "stop" })).status, "stopped")
  assert.equal((await runSshMachine("byom-local", { ...baseRequest, action: "remove" })).status, "removed")
  assert.equal((await runSshMachine("byom-local", { ...twoRequest, action: "remove" })).status, "removed")
  const bad = join(scratch, "tampered.tar.gz"); await writeFile(bad, "corrupt archive")
  await assert.rejects(runSshMachine("byom-local", { ...baseRequest, installId: "byom-tampered" }, { ...release, archive: bad }), /SSH deployment failed/)
  await assert.rejects(runSshMachine("byom-local", { ...baseRequest, action: "start" }), /SSH deployment failed/)
  assert.equal(await readFile(join(staging, "sentinel"), "utf8"), "keep")
  assert.deepEqual(await readdir(join(home, ".local/share/chariox/ssh-machines")), [])
  assert.deepEqual(await readdir(join(home, ".config/systemd/user")), [])
  assert.equal((await readdir(home)).some(n => n.startsWith(".chariox-byom-upload.")), false)
  const calls = (await readFile(join(home, "fixture-service-calls"), "utf8")).trim().split("\n").map(JSON.parse)
  assert.ok(calls.flat().every(arg => !arg.includes("chariox-md-staging")))
})
