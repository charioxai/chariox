/** MP-08 / MP-10: local 998 and encrypted peer 437 completion notifications. */
export interface WorkflowNotificationSource {
  kind: "workflow_exit" | "owner_action" | "workflow_run";
  source_id: string; owner_user_id: string; kernel_id: string;
  session_id: string; workflow_id: string; exit_node_id: string; enabled: boolean; available: boolean;
  name: string; exit_ordinal: number; display_name: string
}
export interface WorkflowNotificationSourceSummary {
  kind: "workflow_exit" | "owner_action" | "workflow_run";
  source_id: string; kernel_id: string; session_id: string; workflow_id: string;
  exit_node_id: string; exit_ordinal: number; name: string; display_name: string; available: boolean
}
export interface WorkflowNotificationSubscription {
  subscription_id: string; source_id: string; owner_user_id: string; created_by_user_id: string;
  target_kind: "workflow_endpoint" | "agent" | "outside_agent"; target_kernel_id: string; source_kernel_id: string; session_id: string; workflow_id: string;
  publication_id: string; endpoint_id: string; queue_id: string; source_available: boolean;
  agent_id?: string; trigger_id?: string
}
export interface WorkflowNotificationEnvelope {
  source_id: string; display_name: string; occurrence_id: string;
  output: { message: string; artifacts?: { id: string; kind: string; path: string; display_name: string }[] };
  ancestry: string[]; deadline_ms: number
}
export type WorkflowNotificationAck = "accepted" | "duplicate" | "expired" | "loop_dropped"
export interface WorkflowNotificationDiagnostic { source_id: string; occurrence_id: string; code: string }
export type WorkflowNotificationSourceRegisteredResponse = { WorkflowNotificationSourceRegistered: { source: WorkflowNotificationSource } }
export type WorkflowNotificationAttachedResponse = { WorkflowNotificationAttached: { subscription: WorkflowNotificationSubscription } }
export type WorkflowNotificationDetachedResponse = { WorkflowNotificationDetached: { subscription_id: string } }
/** What a grant may follow with `SubscribeEvents{sources}` (protocol 1000). */
export type NotificationSourcesResponse = { NotificationSources: { sources: WorkflowNotificationSourceSummary[] } }
export type WorkflowNotificationsResponse = {
  WorkflowNotifications: { sources: WorkflowNotificationSourceSummary[]; subscriptions: WorkflowNotificationSubscription[]; listeners: WorkflowNotificationSubscription[]; diagnostics: WorkflowNotificationDiagnostic[] }
}
