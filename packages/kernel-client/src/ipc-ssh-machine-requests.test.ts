import test from "node:test"
import assert from "node:assert/strict"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { addSshMachineRequest, removeSshMachineRequest } from "./ipc-ssh-machine-requests.js"
test("MP-07/MP-08/MP-11 owner-managed SSH requests require protocol 444 and carry no ticket", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 444)
  assert.deepEqual(addSshMachineRequest("linux-lan", { install_id: "byom-lan-eval", port: 55129, release: "approved" }), { AddSshMachine: { host: "linux-lan", install_id: "byom-lan-eval", port: 55129, release: "approved" } })
  assert.deepEqual(removeSshMachineRequest("byom-lan-eval"), { RemoveSshMachine: { install_id: "byom-lan-eval" } })
})
