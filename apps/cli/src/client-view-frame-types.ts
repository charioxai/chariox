// MP-08 / MP-10 / MP-11: bounded inert frame storage, independent of the native renderer.
type Color = [number, number, number, number]
type CachedSpan = { text: string; width: number; fg: Color; bg: Color; attributes: number }
export type CachedTranscriptFrame = {
  width: number
  height: number
  lines: { spans: CachedSpan[] }[]
}
const integer = (value: number, minimum: number, maximum: number) =>
  Number.isInteger(value) && value >= minimum && value <= maximum
const color = (value: Color) => Array.isArray(value) && value.length === 4
  && value.every(channel => integer(channel, 0, 255))

export function validCachedTranscriptFrame(frame: CachedTranscriptFrame): boolean {
  if (!frame || !integer(frame.width, 1, 1000) || !integer(frame.height, 1, 500)
    || !Array.isArray(frame.lines) || frame.lines.length !== frame.height) return false
  return frame.lines.every(line => line && Array.isArray(line.spans)
    && line.spans.length <= frame.width && line.spans.every(span => span
      && typeof span.text === "string" && span.text.length <= 8192
      && !/[\x00-\x1f\x7f]/.test(span.text)
      && integer(span.width, 1, frame.width) && integer(span.attributes, 0, 255)
      && color(span.fg) && color(span.bg)))
}
