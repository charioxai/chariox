import type { KernelBrowserMirrorAction } from "./browser-mirror-types.js"
// MD-2: protocol 417; user identity comes from transport admission, never this payload.
export type KernelBrowserInput =
  | { kind: "click"; x: number; y: number }
  | { kind: "text"; text: string }
  | { kind: "key"; key: string }
  | { kind: "scroll"; x: number; y: number; delta_x: number; delta_y: number }
export type KernelBrowserCommand =
  | { op: "list_grants" }
  | { op: "subscribe_grants"; after: number; wait_ms: number }
  | { op: "revoke_grants"; agent_id: string | null }
  | { op: "mirror_subscribe"; tab_id: string; generation: number; device_scale_factor: 1 | 2 }
  | { op: "mirror_next"; subscription_id: string; generation: number; after_sequence: number; drift_nodes: string[] }
  | { op: "mirror_close"; subscription_id: string; generation: number }
  | { op: "mirror_input"; tab_id: string; generation: number; document_id: string; subscription_id: string; sequence: number; action: KernelBrowserMirrorAction }
  | { op: "display_capture" | "display_takeover" | "display_release"; tab_id: string; generation: number }
  | { op: "display_actors" }
  | { op: "display_subscribe"; tab_id: string; generation: number; codecs: string[]; bitrate: number; device_scale_factor: 1 | 2 }
  | { op: "display_next"; subscription_id: string; generation: number; after_sequence: number }
  // MP-08/MP-10: protocol 475 push acknowledgement on a relay display subscription.
  | { op: "display_ack"; subscription_id: string; generation: number; sequence: number; lost: boolean }
  | { op: "display_input"; tab_id: string; generation: number; document_id: string; input: KernelBrowserInput }
  | { op: "start" | "state" | "stop" }
  | { op: "open"; url: string }
  | { op: "close" | "snapshot" | "screenshot" | "subscribe"; tab_id: string; generation: number }
  | { op: "navigate"; tab_id: string; generation: number; url: string }
  | { op: "input"; tab_id: string; generation: number; input: KernelBrowserInput }
  | { op: "poll" | "unsubscribe"; subscription_id: string; generation: number }
export type KernelBrowserRequest = { KernelBrowser: { command: KernelBrowserCommand } }
export type KernelBrowserFrame = {
  generation: number; tab_id: string; mime_type: "image/png" | "image/jpeg";
  data_base64: string; width: number; height: number; sequence?: number
}
export type UserDomainWindowAccess = {
  kernel_id: string; kernel_name: string; focused_agent_kernel_id: string | null;
  reachable_by_focused_agent: boolean
}
export type UserDomainResource =
  | { kind: "browser_tab"; tab_id: string }
  | { kind: "app_view"; view_id: string }
  | { kind: "note"; note_id: string }
  | { kind: "capture"; capture_id: string }
export type UserDomainGrant = {
  agent_id: string; session_id: string; kernel_id: string; resources: UserDomainResource[];
  since_ms: number; focused: boolean; idle_since_ms: number | null;
  idle_timeout_seconds: number; expiry_rule: string
}
export type UserDomainGrantEvent = {
  event: "user_domain_grants_changed"; cursor: number; grants: UserDomainGrant[];
  notice: { agent_id: string; resource: UserDomainResource; at_ms: number } | null
}
export function userDomainWindowBadge(access: UserDomainWindowAccess): string | null {
  return access.reachable_by_focused_agent ? null : `Your focused agent can't control this window — focus an agent on kernel ${access.kernel_name}`
}
export type KernelBrowserTab = { tab_id: string; document_id: string; url: string; title: string }
export type KernelBrowserResult = {
  generation?: number; state?: "ready" | "stopped"; tabs?: KernelBrowserTab[];
  tab_id?: string; subscription_id?: string; frame?: KernelBrowserFrame | null;
  snapshot?: unknown; unsubscribed?: boolean;
  kernel_id?: string; kernel_name?: string; focused_agent_kernel_id?: string | null;
  reachable_by_focused_agent?: boolean
} | KernelBrowserFrame | UserDomainGrantEvent
