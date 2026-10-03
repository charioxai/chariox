import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import test from "node:test"

// Opt in with a built image. This exercises the real stopped-container helper,
// including Docker archive ownership, rather than mocking the copy command.
test("stopped browser retirement preserves numeric archive ownership", {
  skip: !process.env.CHARIOX_SLICE_TEST_IMAGE,
}, () => {
  let container
  const docker = (...args) => execFileSync("docker", args, { encoding: "utf8", timeout: 30000 }).trim()
  const instance = "b".repeat(32)
  const processIdentity = { pid: 22, start: "123", group: 22, session: 22, uid: 1001,
    boot: "12345678-1234-1234-1234-123456789012", namespace: "pid:[123]" }
  const record = { version: 1, instance, profile: "/home/slice/profile",
    supervisor: processIdentity, browser: processIdentity }
  const root = "/tmp/chariox-browser-lifecycle-1001"
  try {
    container = docker("create", "--label", "io.chariox.slice.id=archive-test",
      "--label", "io.chariox.slice.owner-kernel-id=archive-kernel",
      "--label", "io.chariox.slice.owner-machine-id=archive-machine",
      "--entrypoint", "/bin/sh", process.env.CHARIOX_SLICE_TEST_IMAGE,
      "-c", 'umask 077; mkdir -m 700 "$1"; printf "%s" "$2" > "$1/$3.json"',
      "archive-test", root, JSON.stringify(record), instance)
    docker("start", "-a", container)
    assert.equal(docker("inspect", "--format", "{{.State.Status}}", container), "exited")
    const helper = fileURLToPath(new URL("./reconcile-stopped-browser-lifetimes.py", import.meta.url))
    const receipt = JSON.parse(execFileSync("python3", [helper, container,
      "archive-test", "archive-kernel", "archive-machine"], { encoding: "utf8", timeout: 60000 }))
    assert.equal(receipt.retired, 1)
    const returned = execFileSync("docker", ["cp", container + ":" + root + "/" + instance + ".retired.json", "-"],
      { timeout: 30000 })
    const header = returned.subarray(0, 512)
    const octal = offset => Number.parseInt(header.subarray(offset, offset + 8).toString().replace(/\0/g, "").trim(), 8)
    assert.equal(octal(108), 1001)
    assert.equal(octal(116), 1001)
    assert.equal(octal(100) & 0o777, 0o600)
    const size = Number.parseInt(header.subarray(124, 136).toString().replace(/\0/g, "").trim(), 8)
    assert.deepEqual(JSON.parse(returned.subarray(512, 512 + size)), record)
    assert.equal(docker("inspect", "--format", "{{.State.Status}}", container), "exited")
  } finally {
    if (container) docker("rm", container)
  }
})
