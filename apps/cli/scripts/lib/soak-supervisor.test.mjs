// MP-08 / MP-10: accept only the exact owned supervisor; retain foreign PID RED.
import assert from "node:assert/strict"
import test from "node:test"
import { includeSoakSupervisor, verifiedSoakSupervisor } from "./soak-supervisor.mjs"
import { networkNamespaceAttribution } from "./browser-computer-soak-runtime.mjs"

const expected = { pid: 1, startedAtTicks: "100", executable: "/usr/local/bin/node", processGroupId: 1 }
function dependencies(overrides = {}) {
  const fields = Array(20).fill("0"); fields[0] = "S"; fields[2] = "1"; fields[19] = "100"
  return {
    identity: JSON.stringify(expected), currentPid: 42,
    read: async candidate => candidate.endsWith("/stat") ? `1 (node) ${fields.join(" ")}`
      : candidate.endsWith("/cmdline") ? "/usr/local/bin/node\0/pilot/supervisor.mjs\0" : "0::/docker/owned\n",
    link: async candidate => candidate.endsWith("/exe") ? "/usr/local/bin/node" : "owned-namespace",
    ...overrides,
  }
}

test("MP-08/MP-10 owned supervisor participates in namespace and resource bounds", async () => {
  const supervisor = await verifiedSoakSupervisor(dependencies())
  const rows = [{ pid: 1, rssKb: 10 }, { pid: 42, rssKb: 20 }, { pid: 99, rssKb: 30 }]
  const measured = includeSoakSupervisor(rows, new Set([42]), supervisor)
  assert.deepEqual(measured.map(row => row.pid), [1, 42])
  assert.equal(measured.reduce((sum, row) => sum + row.rssKb, 0), 30)
  assert.equal(measured[0].scope, "supervisor")
  const attribution = await networkNamespaceAttribution(new Set(measured.map(row => row.pid)), 32, {
    currentPid: 42, listProc: async () => ["1", "42", "99"], readNamespace: async () => "net:[owned]",
  })
  assert.equal(attribution.exclusive, false)
  assert.deepEqual(attribution.foreignPids, [99])
})

test("MP-08/MP-10 reused PID, wrong command and foreign namespace fail closed", async () => {
  await assert.rejects(verifiedSoakSupervisor(dependencies({ identity: JSON.stringify({ ...expected, startedAtTicks: "101" }) })), /identity changed/)
  const base = dependencies()
  await assert.rejects(verifiedSoakSupervisor(dependencies({ read: async candidate => candidate.endsWith("/cmdline") ? "unrelated\0" : base.read(candidate) })), /identity changed/)
  await assert.rejects(verifiedSoakSupervisor(dependencies({ link: async candidate => candidate.includes("/42/") ? "foreign" : base.link(candidate) })), /namespace/)
  await assert.rejects(verifiedSoakSupervisor(dependencies({ identity: JSON.stringify({ ...expected, pid: 99 }) })), /PID 1/)
  assert.equal(await verifiedSoakSupervisor({ identity: "" }), null)
})
