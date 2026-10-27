import assert from "node:assert/strict"
import test from "node:test"
import { defaultKernelUnixSocketPath, kernelSocketConfigRoot } from "./kernel-unix-socket-path.js"

test("maximal slice ID agrees with Rust discovery and normalizes config scope", () => {
  const id = `slice:${"a".repeat(64)}:${"b".repeat(64)}`
  const expected = "/tmp/chariox-42/4445472abcb8084eeec1def5239514b53fa12c0919b849070a897b750ba5f609.sock"
  assert.equal(defaultKernelUnixSocketPath(id, "/var/lib/chariox/slice-private/kernel", 42), expected)
  assert.equal(defaultKernelUnixSocketPath(id, "/var/lib/chariox/other/../slice-private//kernel/.", 42), expected)
  const deepHome = `/var/lib/chariox/${"deep/".repeat(40)}slice-private/kernel`
  const path = defaultKernelUnixSocketPath(id, deepHome, 4294967295)
  assert.ok(Buffer.byteLength(path) < 104)
  assert.notEqual(path, defaultKernelUnixSocketPath(id, `${deepHome}/other`, 4294967295))
  assert.notEqual(path, defaultKernelUnixSocketPath(`${id}c`, deepHome, 4294967295))
  assert.notEqual(path, defaultKernelUnixSocketPath(id, deepHome, 42))
})

test("discovery config scope follows kernel home precedence", () => {
  assert.equal(kernelSocketConfigRoot({ CHARIOX_HOME: "/explicit", XDG_CONFIG_HOME: "/xdg", HOME: "/home" }), "/explicit")
  assert.equal(kernelSocketConfigRoot({ XDG_CONFIG_HOME: "/xdg", HOME: "/home" }), "/xdg/chariox")
  assert.equal(kernelSocketConfigRoot({ HOME: "/home" }), "/home/.chariox")
  assert.equal(kernelSocketConfigRoot({ TMPDIR: "/temp" }), "/temp/chariox/config")
})
