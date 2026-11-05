import test from "node:test"
import assert from "node:assert/strict"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { issueOwnMachineEnrollmentRequest, ownMachineEnrollmentMinimumProtocolVersion } from "./ipc-own-machine-enrollment-requests.js"
test("own machine enrollment is asked by name only and starts at protocol 971", () => {
  assert.equal(ownMachineEnrollmentMinimumProtocolVersion, 971)
  assert.ok(LOCAL_DAEMON_PROTOCOL_VERSION >= ownMachineEnrollmentMinimumProtocolVersion)
  assert.deepEqual(issueOwnMachineEnrollmentRequest(), { IssueOwnMachineEnrollment: { machine_label: null } })
  assert.deepEqual(issueOwnMachineEnrollmentRequest("build box"), { IssueOwnMachineEnrollment: { machine_label: "build box" } })
})
