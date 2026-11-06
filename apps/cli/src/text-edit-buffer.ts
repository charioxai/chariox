// Pure text editing for dialogs that own raw key events (the Environment dialog fields).
export type TextBuffer = { readonly text: string; readonly cursor: number }

export type EditKey = {
  readonly name: string
  readonly sequence?: string | undefined
  readonly ctrl?: boolean | undefined
  readonly meta?: boolean | undefined
  readonly alt?: boolean | undefined
  readonly super?: boolean | undefined
  readonly shift?: boolean | undefined
}

// Pasted text may carry CRLF or terminal control bytes; only newlines and tabs are text.
const CONTROL_CHARACTERS = /[\u0000-\u0008\u000b-\u001f\u007f]/u

export function cleanPastedText(text: string, multiline: boolean): string {
  const unified = text.replace(/\r\n?/gu, "\n").replace(new RegExp(CONTROL_CHARACTERS.source, "gu"), "")
  return multiline ? unified : unified.replace(/\n/gu, "")
}

export function bufferAtEnd(text: string): TextBuffer {
  return { text, cursor: text.length }
}

export function insertText(buffer: TextBuffer, inserted: string): TextBuffer {
  return {
    text: buffer.text.slice(0, buffer.cursor) + inserted + buffer.text.slice(buffer.cursor),
    cursor: buffer.cursor + inserted.length,
  }
}

function previousBoundary(text: string, cursor: number): number {
  if (cursor <= 0) return 0
  const low = text.charCodeAt(cursor - 1)
  const high = cursor >= 2 ? text.charCodeAt(cursor - 2) : 0
  return low >= 0xdc00 && low <= 0xdfff && high >= 0xd800 && high <= 0xdbff ? cursor - 2 : cursor - 1
}

function nextBoundary(text: string, cursor: number): number {
  if (cursor >= text.length) return text.length
  const high = text.charCodeAt(cursor)
  return high >= 0xd800 && high <= 0xdbff && cursor + 1 < text.length ? cursor + 2 : cursor + 1
}

function lineStart(text: string, cursor: number): number {
  return text.lastIndexOf("\n", cursor - 1) + 1
}

function lineEnd(text: string, cursor: number): number {
  const end = text.indexOf("\n", cursor)
  return end === -1 ? text.length : end
}

function moveVertically(buffer: TextBuffer, direction: -1 | 1): TextBuffer {
  const { text, cursor } = buffer
  const start = lineStart(text, cursor)
  const column = cursor - start
  if (direction === -1) {
    if (start === 0) return { text, cursor: 0 }
    const previousStart = lineStart(text, start - 1)
    return { text, cursor: Math.min(previousStart + column, start - 1) }
  }
  const end = lineEnd(text, cursor)
  if (end === text.length) return { text, cursor: text.length }
  const nextStart = end + 1
  return { text, cursor: Math.min(nextStart + column, lineEnd(text, nextStart)) }
}

/** The text a key press types, or "" for keys that do not type. */
export function typedText(key: EditKey): string {
  if (key.ctrl || key.meta || key.alt || key.super) return ""
  if (key.sequence && !CONTROL_CHARACTERS.test(key.sequence) && !/[\n\r\t]/u.test(key.sequence)) return key.sequence
  if (key.name === "space") return " "
  if (Array.from(key.name).length !== 1 || CONTROL_CHARACTERS.test(key.name)) return ""
  return key.shift ? key.name.toLocaleUpperCase() : key.name
}

/** Applies one key to the buffer; null when the key does not edit. */
export function applyEditKey(buffer: TextBuffer, key: EditKey, multiline: boolean): TextBuffer | null {
  if (!key.ctrl && !key.meta && !key.alt && !key.super) {
    switch (key.name) {
      case "left": return { text: buffer.text, cursor: previousBoundary(buffer.text, buffer.cursor) }
      case "right": return { text: buffer.text, cursor: nextBoundary(buffer.text, buffer.cursor) }
      case "up": return multiline ? moveVertically(buffer, -1) : { text: buffer.text, cursor: 0 }
      case "down": return multiline ? moveVertically(buffer, 1) : { text: buffer.text, cursor: buffer.text.length }
      case "home": return { text: buffer.text, cursor: lineStart(buffer.text, buffer.cursor) }
      case "end": return { text: buffer.text, cursor: lineEnd(buffer.text, buffer.cursor) }
      case "backspace": {
        const from = previousBoundary(buffer.text, buffer.cursor)
        return { text: buffer.text.slice(0, from) + buffer.text.slice(buffer.cursor), cursor: from }
      }
      case "delete": {
        const to = nextBoundary(buffer.text, buffer.cursor)
        return { text: buffer.text.slice(0, buffer.cursor) + buffer.text.slice(to), cursor: buffer.cursor }
      }
      case "return":
      case "enter":
        return multiline ? insertText(buffer, "\n") : null
      case "tab":
        return multiline ? insertText(buffer, "\t") : null
    }
  }
  const typed = typedText(key)
  return typed ? insertText(buffer, typed) : null
}
