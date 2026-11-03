/** MP-08 / MP-10: shared TUI attach projection; a subscription has no options. */
import test from "node:test"
import assert from "node:assert/strict"
import { executeWorkflowNotificationTrigger, executeWorkflowNotificationSettings, matchNotificationSource } from "./shell-workflow-notification-command.js"
import type { ShellContext } from "./shell-core.js"
const context: ShellContext = { sessionId: "target", workflowId: "consumer", workspace: "/tmp", worktree: "/tmp", provider: "dev-stub", model: "", effort: "", variables: {} }
const source = { source_id: "source", kernel_id: "peer", session_id: "source-session", workflow_id: "reviewer", exit_node_id: "node-2", exit_ordinal: 2, name: "Reviewer", display_name: "Reviewer-2", available: true }
const publications = { WorkflowPublicationsListed: { publications: [{ id: "publication", workflow_id: "consumer", enabled: true, kind: "event_based" }] } }

test("MP-08 / MP-10: attach uses a discovered source and the ordinary event trigger, with no options", async () => {
  const calls: Record<string, unknown>[] = []
  const deps = { client: { async send(request: Record<string, unknown>) {
    calls.push(request)
    if ("ListWorkflowNotifications" in request) return { WorkflowNotifications: { sources: [source], subscriptions: [], diagnostics: [] } }
    if ("ListWorkflowPublications" in request) return publications
    return { WorkflowNotificationAttached: { subscription: { subscription_id: "sub-1" } } }
  } } }
  const result = await executeWorkflowNotificationTrigger(["attach", "Reviewer-2"], context, deps)
  assert.equal(result.ok, true)
  assert.deepEqual(calls[2], { AttachWorkflowNotification: { session_id: "target", source_id: "source", publication_ref: "publication" } })
  for (const extra of [["--filter", "repo=x"], ["--ttl", "30"], ["--delivery", "inject"], ["success"]]) {
    const refused = await executeWorkflowNotificationTrigger(["attach", "Reviewer-2", ...extra], context, deps)
    assert.equal(refused.ok, false, `attach ${extra.join(" ")} takes no options`)
  }
})
test("MP-08 / MP-10: offline attach is refused", async () => {
  for (const available of [false, true]) {
    let mutations = 0
    const deps = { client: { async send(request: Record<string, unknown>) {
      if ("AttachWorkflowNotification" in request) { mutations++; return { WorkflowNotificationAttached: { subscription: { subscription_id: "s" } } } }
      if ("ListWorkflowPublications" in request) return publications
      return { WorkflowNotifications: { sources: [{ ...source, available }], subscriptions: [], diagnostics: [] } }
    } } }
    const result = await executeWorkflowNotificationTrigger(["attach", "source"], context, deps)
    assert.equal(result.ok, available); assert.equal(mutations, available ? 1 : 0)
  }
})
test("MP-08 / MP-10: a source is named by display name, kernel/display name or id", () => {
  const sources = [source, { ...source, source_id: "other", display_name: "Nightly", kernel_id: "k2" }]
  for (const ref of ["source", "Reviewer-2", "peer/Reviewer-2"]) assert.deepEqual(matchNotificationSource(sources, ref).map(s => s.source_id), ["source"], ref)
  assert.deepEqual(matchNotificationSource(sources, "Missing"), [])
})
test("MP-08 / MP-10: the exit switch registers one exit and takes no options", async () => {
  const calls: Record<string, unknown>[] = []
  const deps = { client: { async send(request: Record<string, unknown>) { calls.push(request); return { WorkflowNotificationSourceRegistered: { source } } } } }
  const result = await executeWorkflowNotificationSettings(["on", "consumer", "exit-2"], context, deps)
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /Reviewer-2/)
  assert.deepEqual(calls, [{ RegisterWorkflowNotificationSource: { session_id: "target", workflow_ref: "consumer", exit_node_id: "exit-2", enabled: true } }])
  for (const args of [["on"], ["on", "consumer"], ["on", "consumer", "exit-2", "--fields", "a"], ["maybe", "consumer", "exit-2"], ["on", "--fields"]]) {
    assert.equal((await executeWorkflowNotificationSettings(args, context, deps)).ok, false, args.join(" "))
  }
  assert.equal(calls.length, 1)
})
test("MP-08 / MP-10: list shows every subscription id and diagnostics so detach is reachable", async () => {
  const subscription = { subscription_id: "sub-1", source_id: "source", source_available: true, target_kind: "workflow_endpoint", publication_id: "publication" }
  const agent = { ...subscription, subscription_id: "sub-3", target_kind: "agent", agent_id: "agent-1" }
  const offline = { ...subscription, subscription_id: "sub-2", source_available: false }
  const deps = { client: { async send(request: Record<string, unknown>) {
    if ("ListWorkflowNotifications" in request) return { WorkflowNotifications: { sources: [source], subscriptions: [subscription, offline, agent], diagnostics: [{ source_id: "source", occurrence_id: "run-9", code: "workflow_notification_loop_dropped" }] } }
    if ("ListWorkflowPublications" in request) return publications
    return { WorkflowNotificationAttached: { subscription } }
  } } }
  const listed = await executeWorkflowNotificationTrigger(["list"], context, deps)
  assert.match(listed.message ?? "", /sub-1/)
  assert.match(listed.message ?? "", /sub-2.*source not available/)
  assert.match(listed.message ?? "", /sub-3.*agent agent-1/)
  assert.match(listed.message ?? "", /run-9.*loop_dropped/)
  assert.match(listed.message ?? "", /Reviewer-2/)
  const attached = await executeWorkflowNotificationTrigger(["attach", "Reviewer-2"], context, deps)
  assert.match(attached.message ?? "", /sub-1/)
})
