import type { CliRenderer, ScrollBoxRenderable } from "@opentui/core"

/** The waiting room view marks its highlighted row, so the scroll box can keep it on screen. */
export const waitingRoomFocusedRowId = "waiting-room-focused-row"

/** Scrolls the least that brings the highlighted waiting-room row into the viewport; call it once the layout is current. */
export function scrollWaitingRoomFocusIntoView(scrollbox: ScrollBoxRenderable): void {
  const row = scrollbox.content.findDescendantById(waitingRoomFocusedRowId)
  if (!row) return
  const top = row.y - scrollbox.content.y
  const bottom = top + row.height
  const visible = scrollbox.viewport.height
  // Rows of the first screenful show the view from its top, with the header art.
  if (bottom <= visible) scrollbox.scrollTo(0)
  else if (top < scrollbox.scrollTop) scrollbox.scrollTo(top)
  else if (bottom > scrollbox.scrollTop + visible) scrollbox.scrollTo(bottom - visible)
}

const pending = new WeakMap<CliRenderer, () => void>()

/** Follows the highlight after the next frame, when the rebuilt view has its layout. */
export function followWaitingRoomFocus(renderer: CliRenderer, scrollbox: () => ScrollBoxRenderable | undefined): void {
  pending.get(renderer)?.()
  const stop = () => {
    renderer.removePostProcessFn(afterLayout)
    pending.delete(renderer)
  }
  const afterLayout = () => {
    stop()
    const box = scrollbox()
    if (!box) return
    const before = box.scrollTop
    scrollWaitingRoomFocusIntoView(box)
    if (box.scrollTop !== before) renderer.requestRender()
  }
  pending.set(renderer, stop)
  renderer.addPostProcessFn(afterLayout)
}
