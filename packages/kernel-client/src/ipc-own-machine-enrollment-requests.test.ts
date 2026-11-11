import test from "node:test"
import assert from "node:assert/strict"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { issueOwnMachineEnrollmentRequest, ownMachineEnrollmentMinimumProtocolVersion } from "./ipc-own-machine-enrollment-requests.js"
test("own machine enrollment names nothing: the machine reports its own name, from protocol 971", () => {
  assert.equal(ownMachineEnrollmentMinimumProtocolVersion, 971)
  assert.ok(LOCAL_DAEMON_PROTOCOL_VERSION >= ownMachineEnrollmentMinimumProtocolVersion)
  assert.deepEqual(issueOwnMachineEnrollmentRequest(), { IssueOwnMachineEnrollment: {} })
})
