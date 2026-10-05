import type { KernelBrowserMirrorAction } from "./browser-mirror-types.js"
// MD-2: protocol 417; user identity comes from transport admission, never this payload.
export type KernelBrowserInput =
  | { kind: "click"; x: number; y: number }
  | { kind: "text"; text: string }
  | { kind: "key"; key: string }
  | { kind: "scroll"; x: number; y: number; delta_x: number; delta_y: number }
export type KernelBrowserCommand =
  | { op: "mirror_subscribe"; tab_id: string; generation: number; device_scale_factor: 1 | 2 }
  | { op: "mirror_next"; subscription_id: string; generation: number; after_sequence: number; drift_nodes: string[] }
  | { op: "mirror_close"; subscription_id: string; generation: number }
  | { op: "mirror_input"; tab_id: string; generation: number; document_id: string; subscription_id: string; sequence: number; action: KernelBrowserMirrorAction }
  | { op: "display_capture" | "display_takeover" | "display_release"; tab_id: string; generation: number }
  | { op: "display_actors" }
  | { op: "display_subscribe"; tab_id: string; generation: number; codecs: string[]; bitrate: number; device_scale_factor: 1 | 2 }
  | { op: "display_next"; subscription_id: string; generation: number; after_sequence: number }
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
export type KernelBrowserTab = { tab_id: string; document_id: string; url: string; title: string }
export type KernelBrowserResult = {
  generation?: number; state?: "ready" | "stopped"; tabs?: KernelBrowserTab[];
  tab_id?: string; subscription_id?: string; frame?: KernelBrowserFrame | null;
  snapshot?: unknown; unsubscribed?: boolean
} | KernelBrowserFrame
