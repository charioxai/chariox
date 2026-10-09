// MP-08/MP-10/MP-11 — DOM mirror v2 wire (local protocol 482); kernel remains browser authority.
export type Mirror2Kind = 'document' | 'shadow' | 'element' | 'text' | 'frame' | 'mask' | 'tile'
export type Mirror2Form = { value: string; checked: boolean; selected_index: number; selection_start: number | null; selection_end: number | null }
export type Mirror2Record = {
  id: string; parent: string | null; kind: Mirror2Kind
  tag?: string; ns?: 'svg' | 'math'; attrs?: Record<string, string>; text?: string; css?: string; res?: string
  form?: Mirror2Form; scroll?: [number, number]; size?: [number, number]; display?: string; reason?: string; adopted?: string[]
}
export type Mirror2Op =
  | { op: 'children'; id: string; children: string[]; nodes: Mirror2Record[] }
  | { op: 'attr'; id: string; name: string; value: string | null }
  | { op: 'text'; id: string; text: string }
  | { op: 'css'; id: string; css: string }
  | { op: 'adopted'; id: string; sheets: string[] }
  | { op: 'form'; id: string; form: Mirror2Form }
  | { op: 'scroll'; id: string; scroll: [number, number] }
  | { op: 'size'; id: string; size: [number, number] }
  | { op: 'res'; id: string; res: string | null }
export type Mirror2Resource = { key: string; resource_id: string; mime_type: string; data_base64: string }
export type Mirror2Tile = { node_id: string; x: number; y: number; width: number; height: number; data_base64: string }
export type Mirror2Selection = { anchor_id: string; anchor_offset: number; focus_id: string; focus_offset: number }
export type Mirror2Packet = {
  wire: 2; subscription_id: string; tab_id: string; generation: number; document_id: string
  sequence: number; base_sequence: number | null; reset: boolean; fallback?: string
  root?: string; nodes?: Mirror2Record[]; ops?: Mirror2Op[]
  scroll: [number, number]; focused: string | null; selection: Mirror2Selection | null
  resources: Mirror2Resource[]; tiles: Mirror2Tile[]; css_width: number; css_height: number; device_scale_factor: 1 | 2
}
export type Mirror2Action =
  | { kind: 'click'; node_id: string; x: number; y: number }
  | { kind: 'scroll'; node_id: string; x: number; y: number; delta_x: number; delta_y: number }
  | { kind: 'scroll_to'; node_id: string | null; x: number; y: number }
  | { kind: 'focus'; node_id: string }
  | { kind: 'text'; text: string; node_id?: string }
  | { kind: 'key'; key: string }
  | { kind: 'selection'; anchor_id: string; anchor_offset: number; focus_id: string; focus_offset: number }
  | { kind: 'composition'; text: string; selection_start: number; selection_end: number; node_id?: string }
