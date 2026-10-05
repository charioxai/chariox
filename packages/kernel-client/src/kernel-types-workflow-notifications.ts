/** MP-08 / MP-10: protocol 437, kernel-private workflow notifications. */
export interface WorkflowNotificationSource {
  source_id: string; owner_user_id: string; kernel_id: string;
  session_id: string; workflow_id: string; enabled: boolean; available: boolean
}
export interface WorkflowNotificationSubscription {
  subscription_id: string; source_id: string; owner_user_id: string;
  target_kernel_id: string; session_id: string; workflow_id: string;
  publication_id: string; endpoint_id: string; queue_id: string;
  ttl_days: number; source_available: boolean
}
export interface WorkflowNotificationEnvelope {
  source_id: string; occurrence_id: string; output: { message: string; artifacts?: { id: string; kind: string; path: string; display_name: string }[] };
  ancestry: string[]; deadline_ms: number
}
export type WorkflowNotificationAck = "accepted" | "duplicate" | "expired"
export interface WorkflowNotificationDiagnostic {
  source_id: string; occurrence_id: string; code: string
}

export type WorkflowNotificationSourceRegisteredResponse = {
  WorkflowNotificationSourceRegistered: { source: WorkflowNotificationSource }
}
export type WorkflowNotificationAttachedResponse = {
  WorkflowNotificationAttached: { subscription: WorkflowNotificationSubscription }
}
export type WorkflowNotificationsResponse = {
  WorkflowNotifications: { sources: WorkflowNotificationSource[]; subscriptions: WorkflowNotificationSubscription[]; diagnostics: WorkflowNotificationDiagnostic[] }
}
