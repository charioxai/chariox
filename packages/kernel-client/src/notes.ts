// MD-N3 / MP-08 / MP-11: shared trusted-host adapter. Cloud owns the overlay UI.
import type { PromptAttachmentPart } from "./kernel-types-provider.js"

export const notesMinimumProtocolVersion = 443
export type NoteWindow =
  | { kind: "kernel_browser"; tab_id: string; generation: number }
  | { kind: "room_browser"; session_id: string; tab_id: string }
  | { kind: "panel"; window_id: string }
  | { kind: "terminal"; session_id: string; window_id: string }
export type NoteTextQuote = { exact: string; prefix: string; suffix: string }
export type NoteBox = { x: number; y: number; width: number; height: number }
export type NoteAnchor = { window: NoteWindow; url: string | null; document_id: string | null; hint: string | null; quote: NoteTextQuote }
export type NoteSelection = { selection_id: string; anchor: NoteAnchor; box_css: NoteBox | null }
export type NoteSummary = { note_id: string; window: NoteWindow; resolved: boolean; anchor_state: string; updated_at_ms: number }
export type NoteRecord = {
  note_id: string; author: string; domain: "agent" | "user"; anchor: NoteAnchor; comment: string
  replies: { author: string; comment: string; created_at_ms: number }[]
  resolved: boolean; anchor_state: "attached" | "missing" | "ambiguous" | "unavailable"
  box_css: NoteBox | null; created_at_ms: number; updated_at_ms: number
}
export type NoteCommand =
  | { op: "capture_selection"; window: NoteWindow }
  | { op: "report_selection"; anchor: NoteAnchor; box_css: NoteBox | null }
  | { op: "create"; selection_id: string; comment: string }
  | { op: "list"; window: NoteWindow }
  | { op: "read" | "resolve" | "reanchor" | "ask"; note_id: string }
  | { op: "reply"; note_id: string; comment: string }
export type NoteResult =
  | { event: "selection_changed"; selection: NoteSelection | null }
  | { event: "note_changed"; note: NoteRecord }
  | { event: "notes_listed"; notes: NoteSummary[] }
  | { event: "prompt_draft"; note_id: string; text: string; attachment: PromptAttachmentPart }
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

// The prompt editor inserts these two values locally. Only its ordinary user
// Send action uses SubmitPrompt and the existing PromptAttachmentPart path.
export function notePromptDraft(result: NoteResult): { text: string; attachments: PromptAttachmentPart[] } {
  if (result.event !== "prompt_draft" || !result.text.startsWith("[Chariox note ") || result.attachment.mime !== "text/plain") {
    throw new Error("MD-N3: expected a kernel note draft")
  }
  return { text: result.text, attachments: [result.attachment] }
}
