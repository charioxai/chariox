import assert from "node:assert/strict"
import test from "node:test"
import { watchCliLauncherParent } from "./cli-launcher-parent.js"

test("launcher parent watch exits after parent death without signalling another PID", async () => {
  let parent = 12345
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => parent, () => { exits += 1 }, () => true)
  try {
    assert.equal(exits, 0)
    parent = 1
    await new Promise((resolve) => setTimeout(resolve, 150))
    assert.equal(exits, 1)
  } finally {
    stop()
  }
})

test("launcher parent watch detects death when the runtime caches its parent PID", () => {
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => 12345, () => { exits += 1 }, (pid) => {
    assert.equal(pid, 12345)
    return false
  })
  stop()
  assert.equal(exits, 1)
})

test("launcher parent watch detects death before entry point starts", () => {
  let exits = 0
  const stop = watchCliLauncherParent("12345", () => 1, () => { exits += 1 })
  stop()
  assert.equal(exits, 1)
})

test("direct Bun starts need no launcher and unsafe launcher PIDs are rejected", () => {
  watchCliLauncherParent(undefined, () => 1, () => assert.fail("direct start exited"))()
  for (const pid of ["0", "1", "-1", "NaN", "undefined", "2.5"]) {
    assert.throws(() => watchCliLauncherParent(pid), /invalid CLI launcher parent PID/)
  }
})
