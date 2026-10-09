/** MP-08 / MP-10: local 452 and encrypted peer 82 completion notifications. */
export type NotificationDeliveryMode = "queue" | "inject"
export type WorkflowNotificationEvents = "success" | "failure" | "both"
export interface WorkflowNotificationSource {
  source_id: string; owner_user_id: string; kernel_id: string;
  session_id: string; workflow_id: string; enabled: boolean; available: boolean;
  name: string; output_fields: string[]
}
export interface WorkflowNotificationSourceSummary {
  source_id: string; kernel_id: string; session_id: string; workflow_id: string;
  name: string; events: WorkflowNotificationEvents; fields: string[]; available: boolean
}
export interface WorkflowNotificationSubscription {
  subscription_id: string; source_id: string; owner_user_id: string;
  target_kind: "workflow_endpoint"; target_kernel_id: string; source_kernel_id: string; session_id: string; workflow_id: string;
  publication_id: string; endpoint_id: string; queue_id: string;
  ttl_days: number; source_available: boolean; events: WorkflowNotificationEvents;
  delivery_mode: NotificationDeliveryMode; filters: Record<string, unknown> | null
}
export interface WorkflowNotificationEnvelope {
  source_id: string; occurrence_id: string;
  output: { message: string; artifacts?: { id: string; kind: string; path: string; display_name: string }[] } | null;
  status: "success" | "failure"; subject: string | null; fields: Record<string, unknown>;
  ancestry: string[]; deadline_ms: number
}
export type WorkflowNotificationAck = "accepted" | "duplicate" | "expired" | "filtered" | "loop_dropped"
export interface WorkflowNotificationDiagnostic { source_id: string; occurrence_id: string; code: string }
export type WorkflowNotificationSourceRegisteredResponse = { WorkflowNotificationSourceRegistered: { source: WorkflowNotificationSource } }
export type WorkflowNotificationAttachedResponse = { WorkflowNotificationAttached: { subscription: WorkflowNotificationSubscription } }
export type WorkflowNotificationDetachedResponse = { WorkflowNotificationDetached: { subscription_id: string } }
export type WorkflowNotificationsResponse = {
  WorkflowNotifications: { sources: WorkflowNotificationSourceSummary[]; subscriptions: WorkflowNotificationSubscription[]; diagnostics: WorkflowNotificationDiagnostic[] }
}
