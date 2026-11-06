/** MP-08 / MP-10: owners configure; kernels capture outputs and derive ancestry. */
export function registerWorkflowNotificationSourceRequest(sessionId: string, workflowRef: string, exitNodeId: string, enabled: boolean) {
  return { RegisterWorkflowNotificationSource: { session_id: sessionId, workflow_ref: workflowRef, exit_node_id: exitNodeId, enabled } }
}
export function attachWorkflowNotificationRequest(sessionId: string, sourceId: string, publicationRef: string) {
  return { AttachWorkflowNotification: { session_id: sessionId, source_id: sourceId, publication_ref: publicationRef } }
}
export function listWorkflowNotificationsRequest(sessionId: string) {
  return { ListWorkflowNotifications: { session_id: sessionId } }
}
export function detachWorkflowNotificationRequest(sessionId: string, subscriptionId: string) {
  return { DetachWorkflowNotification: { session_id: sessionId, subscription_id: subscriptionId } }
}
