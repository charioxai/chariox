import { directTerminalOutput } from "./direct-terminal-output.js"
import { ROOM_TERMINAL_USAGE, parseRoomTerminalCommand, runRoomTerminalCommand } from "./room-terminal-command.js"
import type { TerminalSend } from "./room-terminal-attach.js"

/** The renderer calls that hand the outer terminal to an attach and take it back. */
export type RoomTerminalScreen = {
  suspend(): void
  resume(): void
}

export type RoomTerminalSlashDeps = {
  isAttached(): boolean
  sessionId(): string
  send: TerminalSend
  screen: RoomTerminalScreen
  appendNotice(message: string): void
  flashFooter(message: string, tone: "info" | "error"): void
}

/** `/room terminal ...`: list, open, close or attach a Room terminal from the TUI. */
export async function handleRoomTerminalSlash(deps: RoomTerminalSlashDeps, args: readonly string[]): Promise<void> {
  const command = parseRoomTerminalCommand(args)
  if (typeof command === "string") {
    deps.flashFooter(command.replace("usage: terminal", "usage: /room terminal"), "error")
    return
  }
  if (!deps.isAttached()) {
    deps.flashFooter("Attach to a Room session to use its terminals", "error")
    return
  }
  const message = await runRoomTerminalCommand({
    send: deps.send,
    target: { sessionId: deps.sessionId() },
    input: process.stdin,
    output: directTerminalOutput(),
    async withScreen(attach) {
      deps.screen.suspend()
      try {
        return await attach()
      } finally {
        deps.screen.resume()
      }
    },
  }, command)
  if (command.kind === "list") deps.appendNotice(message)
  else deps.flashFooter(message, "info")
}

export { ROOM_TERMINAL_USAGE }
