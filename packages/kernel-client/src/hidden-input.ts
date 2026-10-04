import { StringDecoder } from "node:string_decoder"

const activeInputs = new WeakSet<NodeJS.ReadStream>()

type HiddenInputOptions = {
  input: NodeJS.ReadStream
  output: NodeJS.WritableStream & { isTTY?: boolean }
  prompt?: string
  // readline does not enable paste framing; full-screen renderers already own it.
  manageBracketedPaste?: boolean
  // Only a mask crosses the rendering boundary, never the input value.
  renderMask?: (mask: string) => void
  signal?: AbortSignal
}

/** Temporarily owns TTY input, excluding readline/renderers and their histories. */
export async function readHiddenInput(options: HiddenInputOptions): Promise<string> {
  const { input, output, signal } = options
  if (!input.isTTY || !output.isTTY || typeof input.setRawMode !== "function") {
    throw new Error("hidden input requires an interactive TTY")
  }
  if (activeInputs.has(input)) throw new Error("secret input is already open")
  if (signal?.aborted) throw new Error("secret input cancelled")

  activeInputs.add(input)
  return await new Promise<string>((resolve, reject) => {
    const wasRaw = Boolean(input.isRaw)
    const wasPaused = input.isPaused()
    const listeners = ["data", "keypress"].map((event) => ({ event, handlers: input.rawListeners(event) }))
    const decoder = new StringDecoder("utf8")
    let value: string[] = []
    let escape = ""
    let pasting = false
    let settled = false

    const finish = (error?: Error) => {
      if (settled) return
      settled = true
      activeInputs.delete(input)
      const result = value.join("")
      value = []
      input.pause()
      input.off("data", onData)
      input.off("end", onEnd)
      input.off("close", onEnd)
      input.off("error", onError)
      output.off("error", onError)
      signal?.removeEventListener("abort", onAbort)
      // Restoration must continue even if the terminal/output has gone away.
      try { input.setRawMode(wasRaw) } catch { error ??= new Error("could not restore terminal input") }
      for (const { event, handlers } of listeners) {
        for (const handler of handlers) input.on(event, handler as (...args: any[]) => void)
      }
      if (!wasPaused) input.resume()
      try {
        options.renderMask?.("")
        if (options.manageBracketedPaste) output.write("\x1b[?2004l")
        if (options.prompt !== undefined) output.write("\n")
      } catch { error ??= new Error("hidden input display failed") }
      if (error) reject(error)
      else resolve(result)
    }
    const onAbort = () => finish(new Error("secret input cancelled"))
    const onEnd = () => finish(new Error("secret input ended before submission"))
    const onError = () => finish(new Error("hidden input stream failed"))
    const onData = (chunk: Buffer | string) => {
      try {
        const text = typeof chunk === "string" ? chunk : decoder.write(chunk)
        for (const char of text) {
          if (escape) {
            // CSI and SS3 may arrive one byte at a time. An ordinary key after
            // ESC is still input; only a recognized control sequence consumes it.
            if (escape === "\x1b" && char !== "[" && char !== "O") escape = ""
            else {
              escape += char
              if (escape === "\x1b[200~") { pasting = true; escape = "" }
              else if (escape === "\x1b[201~") { pasting = false; escape = "" }
              else if (escape.length > 64 || (escape.length > 2 && /[@-~]/.test(char))) escape = ""
              continue
            }
          }
          if (char === "\x1b") { escape = char; continue }
          if (!pasting && char === "\x04") { onEnd(); return }
          if (!pasting && char === "\x03") { onAbort(); return }
          if (!pasting && (char === "\r" || char === "\n")) { finish(); return }
          if (!pasting && (char === "\x7f" || char === "\b")) value.pop()
          else if (!pasting && char === "\x15") value = []
          else if (char >= " " && char !== "\x7f") value.push(char)
          else if (pasting && (char === "\r" || char === "\n" || char === "\t")) value.push(char)
        }
        options.renderMask?.("•".repeat(Math.min(value.length, 64)))
      } catch { finish(new Error("hidden input display failed")) }
    }

    try {
      input.pause()
      for (const { event } of listeners) input.removeAllListeners(event)
      input.setRawMode(true)
      input.on("data", onData)
      input.on("end", onEnd)
      input.on("close", onEnd)
      input.on("error", onError)
      output.on("error", onError)
      signal?.addEventListener("abort", onAbort, { once: true })
      options.renderMask?.("")
      if (options.manageBracketedPaste) output.write("\x1b[?2004h")
      if (options.prompt !== undefined) output.write(options.prompt)
      input.resume()
    } catch { finish(new Error("hidden input could not start")) }
  })
}
