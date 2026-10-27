import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"
import { daemonEnv, remoteRestartRuntimeRoot } from "./live-remote-restart-drill.mjs"

test("remote restart state belongs to a scoped external home and explicitly isolates CHARIOX_HOME", () => {
  const home = "/tmp/chariox-fault-lane-home"
  const rootDir = remoteRestartRuntimeRoot(home, "owned-run")
  assert.ok(rootDir.startsWith(`${home}/.chariox/dev/`))
  const env = daemonEnv({
    baseEnv: { CHARIOX_HOME: "/other-kernel/home" }, rootDir,
    ports: { relayPort: 12345 }, daemonId: "owned-kernel", acceptRemoteLeases: false,
    kernelPort: 12346, mcpPort: 12347, opencodePort: 12348, codexPort: 12349,
  })
  assert.equal(env.CHARIOX_HOME, path.join(rootDir, "owned-kernel", "chariox-home"))
  assert.ok(env.HOME.startsWith(`${rootDir}/owned-kernel/`))
})
