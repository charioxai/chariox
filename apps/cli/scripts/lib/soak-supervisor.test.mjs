// MP-08 / MP-10: accept only the exact owned supervisor; retain foreign PID RED.
import assert from "node:assert/strict"
import test from "node:test"
import { includeSoakSupervisor, verifiedSoakSupervisor } from "./soak-supervisor.mjs"
import { networkNamespaceAttribution } from "./browser-computer-soak-runtime.mjs"

const expected = { supervisor: { pid: 7, startedAtTicks: "101", executable: "/usr/local/bin/node", processGroupId: 7 }, reaper: { pid: 1, startedAtTicks: "100", executable: "/usr/sbin/docker-init", processGroupId: 1 } }
function dependencies(overrides = {}) {
  return {
    identity: JSON.stringify(expected), currentPid: 42,
    read: async candidate => {
      const reaper = candidate.includes("/1/")
      if (candidate.endsWith("/stat")) {
        const fields = Array(20).fill("0"); fields[0] = "S"; fields[1] = reaper ? "0" : "1"; fields[2] = reaper ? "1" : "7"; fields[19] = reaper ? "100" : "101"
        return `${reaper ? 1 : 7} (node) ${fields.join(" ")}`
      }
      return candidate.endsWith("/cmdline") ? `${reaper ? "/sbin/docker-init\0--\0" : ""}/usr/local/bin/node\0/pilot/supervisor.mjs\0` : "0::/docker/owned\n"
    },
    link: async candidate => candidate.endsWith("/exe") ? candidate.includes("/1/") ? "/usr/sbin/docker-init" : "/usr/local/bin/node" : "owned-namespace",
    ...overrides,
  }
}

test("MP-08/MP-10 owned supervisor participates in namespace and resource bounds", async () => {
  const supervisor = await verifiedSoakSupervisor(dependencies())
  const rows = [{ pid: 1, ppid: 0, rssKb: 10 }, { pid: 7, ppid: 1, rssKb: 10 }, { pid: 42, ppid: 7, rssKb: 20 }, { pid: 50, ppid: 1, rssKb: 5 }, { pid: 99, ppid: 0, rssKb: 30 }]
  const measured = includeSoakSupervisor(rows, new Set([42]), supervisor)
  assert.deepEqual(measured.map(row => row.pid), [1, 7, 42, 50])
  assert.equal(measured.reduce((sum, row) => sum + row.rssKb, 0), 45)
  assert.equal(measured[0].scope, "supervisor")
  const attribution = await networkNamespaceAttribution(new Set(measured.map(row => row.pid)), 32, {
    currentPid: 42, listProc: async () => ["1", "7", "42", "50", "99"], readNamespace: async () => "net:[owned]",
  })
  assert.equal(attribution.exclusive, false)
  assert.deepEqual(attribution.foreignPids, [99])
})

test("MP-08/MP-10 reused PID, wrong command and foreign namespace fail closed", async () => {
  await assert.rejects(verifiedSoakSupervisor(dependencies({ identity: JSON.stringify({ ...expected, supervisor: { ...expected.supervisor, startedAtTicks: "999" } }) })), /identity changed/)
  const base = dependencies()
  await assert.rejects(verifiedSoakSupervisor(dependencies({ read: async candidate => candidate.endsWith("/cmdline") ? "unrelated\0" : base.read(candidate) })), /identity changed/)
  await assert.rejects(verifiedSoakSupervisor(dependencies({ link: async candidate => candidate.includes("/42/") ? "foreign" : base.link(candidate) })), /namespace/)
  await assert.rejects(verifiedSoakSupervisor(dependencies({ identity: JSON.stringify({ ...expected, reaper: { ...expected.reaper, pid: 99 } }) })), /PID 1/)
  assert.equal(await verifiedSoakSupervisor({ identity: "" }), null)
})
