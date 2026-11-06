// Shared sizing of the scrolling dialog panels (see cli-dialog-overlay.ts: 2-column padding each side).
export function dialogPageSize(height: number) {
  return Math.max(1, height - Math.max(1, Math.floor(height / 5)) - 4)
}

export function dialogPanelWidth(viewportWidth: number) {
  return Math.min(110, Math.max(30, viewportWidth - 4))
}

export function dialogTextWidth(viewportWidth: number) {
  return Math.max(1, Math.min(dialogPanelWidth(viewportWidth), viewportWidth - 4) - 4)
}
