// MP-08 / MP-10 / MP-11: inert transcript pixels as styled terminal cells, no actions or authority.
import { OptimizedBuffer, RGBA } from "@opentui/core"
import type {CachedTranscriptFrame} from "./client-view-frame-types.js"
export type {CachedTranscriptFrame} from "./client-view-frame-types.js"
type Rect = { x: number; y: number; width: number; height: number }
export function captureTranscriptFrame(buffer: OptimizedBuffer, rect: Rect): CachedTranscriptFrame | undefined {
  const x = Math.max(0, Math.floor(rect.x)), y = Math.max(0, Math.floor(rect.y))
  const width = Math.min(Math.floor(rect.width), buffer.width-x), height = Math.min(Math.floor(rect.height), buffer.height-y)
  if (width <= 0 || height <= 0 || width > 1000 || height > 500) return
  const crop = OptimizedBuffer.create(width, height, buffer.widthMethod)
  try {
    crop.drawFrameBuffer(0, 0, buffer, x, y, width, height)
    const lines = crop.getSpanLines().map(line => ({spans: line.spans.map(span => ({text:span.text,width:span.width,fg:span.fg.toInts(),bg:span.bg.toInts(),attributes:span.attributes & 0xff}))}))
    return {width,height,lines}
  } finally { crop.destroy() }
}
export function paintCachedTranscriptFrame(buffer: OptimizedBuffer, frame: CachedTranscriptFrame, rect: Rect): void {
  buffer.pushScissorRect(rect.x,rect.y,rect.width,rect.height)
  try {
    const start = Math.max(0, frame.height-Math.floor(rect.height))
    for (let row=start;row<frame.lines.length;row++) {
      let x=rect.x
      for (const span of frame.lines[row]!.spans) {
        buffer.drawText(span.text,x,rect.y+row-start,RGBA.fromInts(...span.fg),RGBA.fromInts(...span.bg),span.attributes)
        x+=span.width
        if(x>=rect.x+rect.width)break
      }
    }
  } finally { buffer.popScissorRect() }
}
