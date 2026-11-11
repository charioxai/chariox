import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import { createManagedEnvironment } from "./managed-environment-api.js"
import { WaitingRoomManagedEnvironmentLaunchController } from "./waiting-room-managed-environment-launch-controller.js"

// Cloud's POST /managed-environments route test (apps/api routes/managed-environment-create-request-contract.test.ts) feeds the same
// file to the real route and control service; the Cloud repository also checks that its copy equals this one.
const fixture = JSON.parse(readFileSync(new URL("../src/test/fixtures/managed-environment-create-request.json", import.meta.url), "utf8")) as Record<string, unknown>

test("a new managed machine launch posts exactly the shared create request, with no name", async () => {
  const stop = new Error("request captured")
  const posts: { url: string; body: unknown }[] = []
  const profile = {
    apiUrl: "http://127.0.0.1:44123", accountId: "account-1", userId: "owner-1",
    authenticatedFetch: async (url: string | URL, options?: RequestInit) => { posts.push({ url: String(url), body: JSON.parse(String(options?.body)) }); throw stop },
  } as CloudControlProfile
  const controller = new WaitingRoomManagedEnvironmentLaunchController({
    createEnvironment: input => createManagedEnvironment(profile, input),
    createIdempotencyKey: () => "create-request-1",
  } as never)
  await assert.rejects(controller.prepare({
    kind: "new", region: "hel1", computeClass: "agent-small", managedRepositoryRoot: "/home/chariox",
    shutdownPolicy: { kind: "last_work_finished", warningSeconds: 30 },
    contextPlan: { sourceTargetId: null, kernelContext: "empty", developmentSetup: { kind: "empty" }, providerAccounts: { kind: "none" }, gitCredentials: { kind: "none" } },
  } as never, { assertActive() {}, environmentChanged() {}, progress() {} }), error => error === stop)
  assert.equal(posts.length, 1)
  assert.equal(new URL(posts[0]!.url).pathname, "/managed-environments")
  assert.deepEqual(posts[0]!.body, fixture)
})
