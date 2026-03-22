import { spawn } from "node:child_process"
import process from "node:process"

type ClipboardRenderer = {
  copyToClipboardOSC52(text: string): boolean
}

export async function copyTextToClipboard(text: string, renderer: ClipboardRenderer) {
  if (!text) {
    return
  }

  const copiedViaOsc52 = renderer.copyToClipboardOSC52(text)

  try {
    await copyTextNatively(text)
  } catch (error) {
    if (copiedViaOsc52) {
      return
    }
    throw error
  }
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

    child.once("error", reject)
    child.once("close", (code) => {
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
