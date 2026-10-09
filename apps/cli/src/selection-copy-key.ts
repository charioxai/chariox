// MP-08 / MP-10: share copy recognition between decoded capture and raw replay.
export type SelectionCopyKey = {
  name: string
  sequence?: string
  eventType?: string
  meta?: boolean
  ctrl?: boolean
  shift?: boolean
}

export function isSelectionCopyKey(event: SelectionCopyKey): boolean {
  return event.eventType !== "release"
    && (event.name === "f6" || (event.name === "c" && (Boolean(event.meta) || Boolean(event.ctrl && event.shift))))
}
