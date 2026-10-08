import { spawn } from "node:child_process"
import process from "node:process"
import { isSshTerminal, requestTerminalCopy } from "./terminal-copy.js"

type ClipboardRenderer = {
  copyToClipboardOSC52(text: string): boolean
}

export type ClipboardCopyResult = "copied" | "requested" | "unavailable"

export function clipboardCopyMessage(result: ClipboardCopyResult): string {
  if (result === "copied") return "copied to local clipboard"
  if (result === "requested") return "clipboard request sent (OSC 52, unconfirmed); if empty, use native selection and Copy"
  return "clipboard unavailable; use native selection and your terminal's Copy command"
}

export async function copyTextToClipboard(
  text: string,
  renderer: ClipboardRenderer,
  options: {
    remote?: boolean
    nativeCopy?: (text: string) => Promise<void>
    terminalCopy?: (text: string) => boolean
  } = {},
): Promise<ClipboardCopyResult> {
  // A clipboard helper on the SSH host cannot confirm the user's clipboard.
  if (!(options.remote ?? isSshTerminal())) {
    try {
      await (options.nativeCopy ?? copyTextNatively)(text)
      return "copied"
    } catch { /* Try the terminal transport, retaining an honest fallback. */ }
  }
  try {
    if (renderer.copyToClipboardOSC52(text) || (options.terminalCopy ?? requestTerminalCopy)(text)) {
      return "requested"
    }
  } catch { /* Terminal failure must not be reported as a successful copy. */ }
  return "unavailable"
}

async function copyTextNatively(text: string) {
  if (process.platform === "darwin") {
    await runClipboardCommand("pbcopy", [], text)
    return
  }

  if (process.platform === "win32") {
    await runClipboardCommand("clip.exe", [], text)
    return
  }

  if (!process.env.WAYLAND_DISPLAY && !process.env.DISPLAY) {
    throw new Error("No local desktop clipboard")
  }

  const commands: Array<[string, string[]]> = process.env.WAYLAND_DISPLAY
    ? [
        ["wl-copy", []],
        ["xclip", ["-selection", "clipboard"]],
        ["xsel", ["--clipboard", "--input"]],
      ]
    : [
        ["xclip", ["-selection", "clipboard"]],
        ["xsel", ["--clipboard", "--input"]],
        ["wl-copy", []],
      ]

  let lastError: unknown = new Error("No clipboard command available")
  for (const [command, args] of commands) {
    try {
      await runClipboardCommand(command, args, text)
      return
    } catch (error) {
      lastError = error
    }
  }

  throw lastError
}

function runClipboardCommand(command: string, args: string[], text: string) {
  return new Promise<void>((resolve, reject) => {
    const child = spawn(command, args, {
      stdio: ["pipe", "ignore", "ignore"],
    })

    const timer = setTimeout(() => {
      // This exact child belongs to this invocation; never signal a system PID.
      if (Number.isInteger(child.pid) && child.pid! > 1) child.kill("SIGKILL")
      reject(new Error("clipboard helper timed out"))
    }, 2_000)
    child.once("error", (error) => { clearTimeout(timer); reject(error) })
    child.once("close", (code) => {
      clearTimeout(timer)
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`${command} exited with code ${code ?? "unknown"}`))
    })

    child.stdin?.once("error", reject)
    child.stdin?.end(text)
  })
}
