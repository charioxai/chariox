import { writeSync } from "node:fs"

/** OSC 52 is a request, never an acknowledgement from the user's clipboard. */
export function requestTerminalCopy(text: string): boolean {
  if (!process.stdout.isTTY || process.env.TERM === "dumb") return false
  try {
    writeSync(process.stdout.fd, `\x1b]52;c;${Buffer.from(text, "utf8").toString("base64")}\x07`)
    return true
  } catch {
    return false
  }
}

export function isSshTerminal(): boolean {
  return Boolean(process.env.SSH_CONNECTION || process.env.SSH_CLIENT || process.env.SSH_TTY)
}
