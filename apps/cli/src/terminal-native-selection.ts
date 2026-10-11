import { writeSync } from "node:fs"
import type { CliRenderer, OptimizedBuffer } from "@opentui/core"
import { TUI_MOUSE_ENABLED } from "./terminal-mouse-policy.js"

/** MP-08/MP-10: painted spaces remain literal spaces in the terminal clipboard.
 * Erase only uniform blank row tails after the synchronous native frame, so
 * native Copy gets text and newlines rather than full-screen padding. Cursor,
 * attributes and the visible background are preserved. */
function erasePaintedPadding(buffer: OptimizedBuffer): string {
  const { char, bg } = buffer.buffers
  const rows: string[] = []
  for (let y = 0; y < buffer.height; y++) {
    const last = (y + 1) * buffer.width - 1
    if (char[last] !== 32) continue
    let x = buffer.width - 1
    while (x >= 0 && char[y * buffer.width + x] === 32) {
      const index = (y * buffer.width + x) * 4
      const tail = last * 4
      if (bg[index] !== bg[tail] || bg[index + 1] !== bg[tail + 1]
        || bg[index + 2] !== bg[tail + 2] || bg[index + 3] !== bg[tail + 3]) break
      x--
    }
    if (x === buffer.width - 1) continue
    const color = [0, 1, 2].map(channel => Math.round(bg[last * 4 + channel]! * 255))
    rows.push(`\x1b[${y + 1};${x + 2}H\x1b[0m\x1b[48;2;${color.join(";")}m\x1b[K`)
  }
  // Cursor save/restore does not restore OSC 8. These control-only erasures
  // must retain the renderer's active link so its next frame stays in sync.
  return rows.length ? `\x1b7${rows.join("")}\x1b8` : ""
}

export function installTerminalNativeSelection(renderer: CliRenderer): () => void {
  if (TUI_MOUSE_ENABLED || !process.stdout.isTTY) return () => {}
  // OpenTUI 0.1.87 has no after-frame hook. Keep this pinned renderer adapter
  // beside the native capability adapter; useThread=false ensures output order.
  const native = renderer as unknown as { renderNative(): void; nextRenderBuffer: OptimizedBuffer }
  const original = native.renderNative
  const render = function (this: typeof native) {
    const padding = erasePaintedPadding(this.nextRenderBuffer)
    original.call(this)
    if (padding) writeSync(process.stdout.fd, padding)
  }
  native.renderNative = render
  return () => { if (native.renderNative === render) native.renderNative = original }
}
