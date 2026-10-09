import assert from "node:assert/strict"
import test from "node:test"
import { registerWorkflowNotificationSourceRequest, attachWorkflowNotificationRequest, listWorkflowNotificationsRequest } from "./ipc-workflow-notification-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MP-08 / MP-10 protocol 437 notification contracts are owner/ancestry neutral", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 484)
  assert.deepEqual(registerWorkflowNotificationSourceRequest("s", "w", true), { RegisterWorkflowNotificationSource: { session_id: "s", workflow_ref: "w", enabled: true, output_fields: null } })
  assert.deepEqual(attachWorkflowNotificationRequest("target", "source", "publication"), { AttachWorkflowNotification: { session_id: "target", source_id: "source", publication_ref: "publication", queue_ref: null, ttl_days: 7, events: "success", filters: null, delivery_mode: "queue" } })
  assert.deepEqual(listWorkflowNotificationsRequest("s"), { ListWorkflowNotifications: { session_id: "s" } })
})

 test("MP-08 / MP-10 attach carries inject and opaque dotted filters", () => {
  assert.equal(attachWorkflowNotificationRequest("s", "src", "pub", null, 7, "both", { "opaque.category": ["a", "b"] }, "inject").AttachWorkflowNotification.delivery_mode, "inject")
 })
