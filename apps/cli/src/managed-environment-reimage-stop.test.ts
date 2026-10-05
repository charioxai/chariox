import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import { validateManagedReimageStop, prepareManagedEnvironmentReimageStop } from "./managed-environment-reimage-stop.js"

const profile = {accountId: "account", userId: "owner", apiUrl: "http://127.0.0.1:44123"} as CloudControlProfile
const request = {environmentId: "environment", expectedGeneration: 3, idempotencyKey: "reimage"} as Parameters<typeof prepareManagedEnvironmentReimageStop>[1]
function admission() {
  return {environment: {environmentId: "environment", accountId: "account", runtimeGeneration: 3, desiredState: "stopped", observedState: "stopped", desiredRevision: 8, observedRevision: 8}, operation: {operationId: "stop", environmentId: "environment", requestedByUserId: "owner", kind: "stop", status: "succeeded", idempotencyKey: `reimage-stop:${createHash("sha256").update("reimage").digest("hex")}`, requestDigest: `sha256:${"a".repeat(64)}`, desiredRevision: 8, completedAt: "2026-10-05T00:00:00Z"}}
}
type Admission = Parameters<typeof validateManagedReimageStop>[0]

// MP-08 / MP-11: moving human HTTP preserves Cloud-owned STOP fencing.
test("managed STOP requires one exact converged generation and operation", () => {
  const held = admission(); held.operation.status = "running"; held.environment.observedState = "stopping"
  const pending = validateManagedReimageStop(held as Admission, profile, request)
  assert.equal(pending.done, false)
  assert.equal(validateManagedReimageStop(admission() as Admission, profile, request, pending.binding).done, true)
  for (const [section, field, value] of [
    ["environment", "environmentId", "foreign"], ["environment", "accountId", "foreign"], ["environment", "runtimeGeneration", 4],
    ["environment", "desiredState", "running"], ["environment", "observedState", "stopping"], ["environment", "observedRevision", 7],
    ["operation", "environmentId", "foreign"], ["operation", "requestedByUserId", "foreign"], ["operation", "operationId", "replacement"],
    ["operation", "idempotencyKey", "replacement"], ["operation", "requestDigest", `sha256:${"b".repeat(64)}`],
    ["operation", "desiredRevision", 9], ["operation", "status", "failed"], ["operation", "completedAt", null],
  ] as const) {
    const changed = admission() as any; changed[section][field] = value
    assert.throws(() => validateManagedReimageStop(changed, profile, request, pending.binding), /Cloud|Managed/, `${section}.${field}`)
  }
})

test("managed reimage replay retains original key, next generation and STOP digest", () => {
  const binding = validateManagedReimageStop(admission() as Admission, profile, request).binding
  const replay = admission(); replay.operation.kind = "reimage"; replay.operation.idempotencyKey = "reimage"; replay.environment.runtimeGeneration = 4
  assert.equal(validateManagedReimageStop(replay as Admission, profile, request, binding).done, true)
  replay.operation.requestDigest = `sha256:${"b".repeat(64)}`
  assert.throws(() => validateManagedReimageStop(replay as Admission, profile, request, binding), /unrelated reimage replay/)
})

test("missing STOP admission fails without a lifecycle fallback or reimage POST", async () => {
  const paths: string[] = []
  const authority = {...profile, authenticatedFetch: async (input: string | URL) => {paths.push(new URL(input).pathname); return Response.json({error: {message: "not supported"}}, {status: 404})}}
  await assert.rejects(prepareManagedEnvironmentReimageStop(authority, request), /not supported/)
  assert.deepEqual(paths, ["/managed-environments/environment/reimage/stop"])
})
