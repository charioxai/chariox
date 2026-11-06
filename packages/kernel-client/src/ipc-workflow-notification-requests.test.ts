import assert from "node:assert/strict"
import test from "node:test"
import { registerWorkflowNotificationSourceRequest, attachWorkflowNotificationRequest, listWorkflowNotificationsRequest } from "./ipc-workflow-notification-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MP-08 / MP-10 protocol 437 notification contracts are owner/ancestry neutral", () => {

  assert.deepEqual(registerWorkflowNotificationSourceRequest("s", "w", "exit-1", true), { RegisterWorkflowNotificationSource: { session_id: "s", workflow_ref: "w", exit_node_id: "exit-1", enabled: true } })
  assert.deepEqual(attachWorkflowNotificationRequest("target", "source", "publication"), { AttachWorkflowNotification: { session_id: "target", source_id: "source", publication_ref: "publication" } })
  assert.deepEqual(listWorkflowNotificationsRequest("s"), { ListWorkflowNotifications: { session_id: "s" } })
})

