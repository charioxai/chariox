import type { WorkflowNotificationEvents } from "./kernel-types-workflow-notifications.js"
/** MP-08 / MP-10: owners configure; kernels derive outputs, subjects and ancestry. */
export function registerWorkflowNotificationSourceRequest(sessionId: string, workflowRef: string, enabled: boolean, outputFields: string[] | null = null) {
  return { RegisterWorkflowNotificationSource: { session_id: sessionId, workflow_ref: workflowRef, enabled, output_fields: outputFields } }
}
export function attachWorkflowNotificationRequest(sessionId: string, sourceId: string, publicationRef: string, queueRef: string | null = null, ttlDays = 7, events: WorkflowNotificationEvents = "both", filters: Record<string, unknown> | null = null) {
  return { AttachWorkflowNotification: { session_id: sessionId, source_id: sourceId, publication_ref: publicationRef, queue_ref: queueRef, ttl_days: ttlDays, events, filters } }
}
export function listWorkflowNotificationsRequest(sessionId: string) {
  return { ListWorkflowNotifications: { session_id: sessionId } }
}
export function detachWorkflowNotificationRequest(sessionId: string, subscriptionId: string) {
  return { DetachWorkflowNotification: { session_id: sessionId, subscription_id: subscriptionId } }
}
