// MP-08 / MP-11: shared shell account operations never use kernel credentials.
import assert from "node:assert/strict"
import test from "node:test"
import { createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
test("MP-08 / MP-11 shared shell uses the caller Cloud account for collaboration", async () => {
  const requests: unknown[] = []
  const deps = {client:{send:async () => {throw new Error("kernel must not receive human Cloud controls")}}, cloudAccountClient:{send:async (request: unknown) => {requests.push(request); return {CloudCollaboratorsListed:{collaborators:[]}}}}}
  const result = await executeShellCommand(parseShellCommand("cloud collaborators"), createDefaultShellContext(), deps as any)
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{ListCloudCollaborators:null}])
})
test("MP-08 / MP-11 shared shell diagnoses missing human login before creating an invite", async () => {
  let requests = 0
  const result = await executeShellCommand(parseShellCommand("cloud invite create"), createDefaultShellContext({sessionId:"s"}), {client:{send:async () => {requests++; return {}}}})
  assert.equal(result.ok, false)
  assert.match(result.message!, /client Cloud account/)
  assert.equal(requests, 0)
})

test("MP-08 / MP-11 a signed-out private adapter refuses before creating a local invite", async () => {
  let kernelRequests = 0
  const deps = {client:{send:async () => {kernelRequests++; throw new Error("local invite must not be created")}}, cloudAccountClient:{ensureAuthenticated:async () => false, send:async () => ({})}}
  const result = await executeShellCommand(parseShellCommand("cloud invite create"), createDefaultShellContext({sessionId:"s"}), deps as any)
  assert.equal(result.ok, false)
  assert.match(result.message!, /client Cloud account/)
  assert.equal(kernelRequests, 0)
})
