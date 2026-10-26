// MD-N3 / MP-08 / MP-11: shared trusted-host adapter. Cloud owns the overlay UI.

export const notesMinimumProtocolVersion = 443
export type NoteWindow =
  | { kind: "kernel_browser"; tab_id: string; generation: number }
  | { kind: "room_browser"; session_id: string; tab_id: string }
  | { kind: "panel"; window_id: string }
  | { kind: "terminal"; session_id: string; window_id: string }
export type NoteTextQuote = { exact: string; prefix: string; suffix: string }
export type NoteBox = { x: number; y: number; width: number; height: number }
export type NoteAnchor = { window: NoteWindow; url: string | null; document_id: string | null; hint: string | null; quote: NoteTextQuote }
export type NoteSelection = { anchor: NoteAnchor; box_css: NoteBox | null }
export type NoteCommand =
  | { op: "capture_selection"; window: NoteWindow }
  | { op: "report_selection"; anchor: NoteAnchor; box_css: NoteBox | null }
export type NoteResult = { event: "selection_changed"; selection: NoteSelection | null }
export type NotesResponse = { Notes: { result: NoteResult } }

export function notesRequest(command: NoteCommand, protocolVersion: number) {
  if (!Number.isSafeInteger(protocolVersion) || protocolVersion < notesMinimumProtocolVersion) {
    throw new Error("MD-N3: notes require kernel protocol 443; upgrade this kernel")
  }
  return { Notes: { command } }
}

// Map page CSS geometry into the host overlay. The display client supplies the
// actual rendered page rectangle (after App panel/browser-bar/letterbox offsets).
export function noteOverlayBox(box: NoteBox, page: { width: number; height: number }, rendered: NoteBox): NoteBox | null {
  if (![box.x, box.y, box.width, box.height, page.width, page.height, rendered.x, rendered.y, rendered.width, rendered.height].every(Number.isFinite)
    || [box.width, box.height, page.width, page.height, rendered.width, rendered.height].some(n => n <= 0)) return null
  const left = Math.max(0, box.x), top = Math.max(0, box.y)
  const right = Math.min(page.width, box.x + box.width), bottom = Math.min(page.height, box.y + box.height)
  if (right <= left || bottom <= top) return null
  const scaleX = rendered.width / page.width, scaleY = rendered.height / page.height
  return { x: rendered.x + left * scaleX, y: rendered.y + top * scaleY, width: (right - left) * scaleX, height: (bottom - top) * scaleY }
}
