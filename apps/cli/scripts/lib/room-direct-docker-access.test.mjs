import assert from "node:assert/strict"
import test from "node:test"
import { verifyDirectDockerEngineAccess } from "./room-direct-docker-access.mjs"

const imageId = `sha256:${"a".repeat(64)}`

test("macOS checks the actual engine bind mount with the exact image and bounded container", async () => {
  const calls = []
  const runCommand = async (...args) => { calls.push(args); return { code: 0, stderr: "" } }
  await verifyDirectDockerEngineAccess({
    target: "/private/var/tmp/fixture", writable: true, platform: "darwin", imageId, runCommand,
  })
  assert.equal(calls.length, 1)
  const [program, args, timeout] = calls[0]
  assert.equal(program, "docker")
  assert.equal(timeout, 30_000)
  assert.deepEqual(args.slice(0, 4), ["run", "--rm", "--pull", "never"])
  assert.ok(args.includes("--network") && args.includes("none"))
  assert.ok(args.includes("--read-only") && args.includes("--cap-drop") && args.includes("ALL"))
  assert.ok(args.includes("--memory") && args.includes("128m"))
  assert.ok(args.includes("--cpus") && args.includes("0.25"))
  assert.ok(args.includes("--user") && args.includes("slice"))
  assert.ok(args.includes("type=bind,source=/private/var/tmp/fixture,target=/chariox-drill-access"))
  assert.equal(args.at(-3), imageId)
  assert.equal(args.at(-1), "test -x /chariox-drill-access && test -w /chariox-drill-access")
})

test("macOS read-only root check never pulls or writes the source path", async () => {
  const calls = []
  await verifyDirectDockerEngineAccess({
    target: "/private/var/tmp", writable: false, platform: "darwin", imageId,
    runCommand: async (...args) => { calls.push(args); return { code: 0, stderr: "" } },
  })
  assert.ok(calls[0][1].includes("type=bind,source=/private/var/tmp,target=/chariox-drill-access,readonly"))
  assert.equal(calls[0][1].at(-1), "test -x /chariox-drill-access")
})

test("macOS fails closed without an exact image or when the engine cannot bind the path", async () => {
  await assert.rejects(verifyDirectDockerEngineAccess({
    target: "/private/var/tmp", writable: false, platform: "darwin", imageId: null,
    runCommand: async () => { throw new Error("must not run") },
  }), /requires an exact prebuilt slice image/)
  await assert.rejects(verifyDirectDockerEngineAccess({
    target: "/private/var/tmp", writable: false, platform: "darwin", imageId,
    runCommand: async () => ({ code: 125, stderr: "bind source path does not exist" }),
  }), /cannot traverse.*bind source path does not exist/)
})

test("Linux retains the managed rootless Docker user checks", async () => {
  const calls = []
  await verifyDirectDockerEngineAccess({
    target: "/var/tmp/fixture", writable: true, platform: "linux", managedRootlessEngine: true, imageId: null,
    runCommand: async (...args) => { calls.push(args); return { code: 0 } },
  })
  assert.deepEqual(calls, [
    ["runuser", ["-u", "chariox-docker", "--", "test", "-x", "/var/tmp/fixture"], 10_000],
    ["runuser", ["-u", "chariox-docker", "--", "test", "-w", "/var/tmp/fixture"], 10_000],
  ])
})

test("Linux local Docker checks the bind in the engine without a managed service user", async () => {
  const calls = []
  await verifyDirectDockerEngineAccess({
    target: "/private/var/tmp/fixture", writable: true, platform: "linux", imageId,
    runCommand: async (...args) => { calls.push(args); return { code: 0, stderr: "" } },
  })
  assert.equal(calls.length, 1)
  assert.equal(calls[0][0], "docker")
  assert.ok(calls[0][1].includes("type=bind,source=/private/var/tmp/fixture,target=/chariox-drill-access"))
})
