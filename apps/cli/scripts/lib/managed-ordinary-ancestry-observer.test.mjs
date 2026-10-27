import assert from "node:assert/strict"
import test from "node:test"
import { observeExecutableAncestry } from "./managed-ordinary-ancestry-observer.mjs"

function stat(pid, parent, ticks = "9001") {
  return `${pid} (fixture) ${["S", parent, ...Array(17).fill("0"), ticks].join(" ")}\n`
}

function fixture({ parent = 1, denied = false, changed = false } = {}) {
  let reads = 0
  return {
    readFile: async path => {
      if (path === "/proc/7/stat") return stat(7, parent, changed && ++reads > 1 ? "9002" : "9001")
      if (path === "/proc/1/stat") return stat(1, 0)
      throw new Error("MP-01 unexpected metadata read")
    },
    readlink: async path => {
      if (denied && path === "/proc/7/exe") throw Object.assign(new Error("denied"), {code: "EACCES"})
      if (path === "/proc/7/exe") return "/usr/bin/bwrap"
      if (path === "/proc/1/exe") return "/usr/lib/systemd/systemd"
      throw new Error("MP-01 unexpected executable read")
    },
  }
}

test("MP-01 producer observes executable bwrap ancestry and excludes command lines/environment", async () => {
  const chain = await observeExecutableAncestry({pid: 7, filesystem: fixture()})
  assert.equal(chain[0].executable_basename, "bwrap")
  assert.equal(chain.at(-1).pid, 1)
  assert.deepEqual(Object.keys(chain[0]).sort(), ["pid", "parentPid", "startTimeTicks", "executable_basename", "executable_path_sha256"].sort())
})

test("MP-01 producer fails closed on cycles, truncated ancestry and PID reuse", async () => {
  for (const options of [{parent: 7}, {parent: 0}, {changed: true}]) {
    await assert.rejects(observeExecutableAncestry({pid: 7, filesystem: fixture(options)}))
  }
})

test("MP-01/MP-10 nondumpable ancestor uses read-only metadata with exact identity binding", async () => {
  let calls = 0
  const chain = await observeExecutableAncestry({pid: 7, filesystem: fixture({denied: true}),
    privilegedMetadata: async (action, pid) => {
      assert.equal(action, "process"); assert.equal(pid, 7); calls++
      return {pid, stat: stat(pid, 1), executableLink: "/usr/bin/codex"}
    },
  })
  assert.equal(calls, 1)
  assert.equal(chain[0].executable_basename, "codex")
  await assert.rejects(observeExecutableAncestry({pid: 7, filesystem: fixture({denied: true}),
    privilegedMetadata: async () => ({pid: 7, stat: stat(7, 1, "9002"), executableLink: "/usr/bin/codex"}),
  }), /changed/)
})

test("MP-01 local Linux process metadata reaches PID 1 without exposing arguments", async () => {
  const chain = await observeExecutableAncestry()
  assert.equal(chain[0].pid, process.pid)
  assert.equal(chain.at(-1).pid, 1)
})
