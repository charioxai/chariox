/** MP-08 / MP-10 protocol 437. Owner and ancestry are kernel-derived. */
export function registerWorkflowNotificationSourceRequest(sessionId: string, workflowRef: string, enabled: boolean) {
  return { RegisterWorkflowNotificationSource: { session_id: sessionId, workflow_ref: workflowRef, enabled } }
}
export function attachWorkflowNotificationRequest(sessionId: string, sourceId: string, publicationRef: string, queueRef: string | null = null, ttlDays = 7) {
  return { AttachWorkflowNotification: { session_id: sessionId, source_id: sourceId, publication_ref: publicationRef, queue_ref: queueRef, ttl_days: ttlDays } }
}
export function listWorkflowNotificationsRequest(sessionId: string) {
  return { ListWorkflowNotifications: { session_id: sessionId } }
}
