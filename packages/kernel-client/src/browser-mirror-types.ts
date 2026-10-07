// MP-08/MP-10/MP-11 — local protocol 443; kernel remains browser authority.
import type { KernelBrowserInput } from './kernel-types-kernel-browser.js'
export type KernelBrowserMirrorAction =
  | { kind: 'click' | 'focus'; node_id: string }
  | { kind: 'text'; node_id: string; text: string }
  | { kind: 'scroll'; node_id: string; delta_x: number; delta_y: number }
  | { kind: 'key'; key: string }
  | { kind: 'selection'; anchor_id: string; anchor_offset: number; focus_id: string; focus_offset: number }
  | { kind: 'composition'; node_id: string; text: string; selection_start: number; selection_end: number }
  | { kind: 'coordinate'; input: KernelBrowserInput }
export type MirrorBox = { x: number; y: number; width: number; height: number }
export type MirrorNode = {
  id: string; parent: string | null; children: string[];
  kind: 'element' | 'text' | 'shadow' | 'document' | 'frame' | 'mask' | 'tile';
  tag?: string; text?: string; attributes?: Record<string,string>; style?: Record<string,string>;
  box?: MirrorBox; resource?: string; reason?: string; scroll?: { x: number; y: number };
  form?: { value: string; checked: boolean; selected_index: number; selection_start: number | null; selection_end: number | null };
  pseudo?: Record<string,{ text: string; style: Record<string,string> }>;
}
export type MirrorResource = { resource_id: string; mime_type: string; data_base64: string }
export type MirrorFont = { family: string; weight: string; style: string; resource: string }
export type MirrorTile = { node_id: string; x: number; y: number; width: number; height: number; data_base64: string }
export type MirrorSelection = { anchor_id: string; anchor_offset: number; focus_id: string; focus_offset: number }
export type MirrorPacket = {
  subscription_id: string; tab_id: string; generation: number; document_id: string;
  sequence: number; base_sequence: number | null; reset: boolean; hash: string;
  root: string; nodes: MirrorNode[]; removed: string[]; resources: MirrorResource[];
  fonts: MirrorFont[]; tiles: MirrorTile[]; scroll: { x: number; y: number }; focused: string | null; selection: MirrorSelection | null;
  css_width: number; css_height: number; device_scale_factor: 1 | 2;
}
