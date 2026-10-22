import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { resolveSessionRequest } from "@chariox/kernel-client/ipc-requests"
import type { RuntimeSession } from "@chariox/kernel-client/kernel-types"

import { parseRoomTerminalCommand, ROOM_TERMINAL_USAGE, runRoomTerminalCommand } from "./room-terminal-command.js"

const USAGE = `usage: chariox terminal --session SESSION [--kernel-url URL] [list | new [TITLE] | close TERMINAL | TERMINAL]
Attaches this terminal to a Room terminal (the newest running one, else a new one). Ctrl-] detaches.`

export function parseTerminalArgs(argv: readonly string[]): { session: string; kernelUrl?: string; rest: string[] } | string {
  const rest: string[] = []
  let session: string | undefined
  let kernelUrl: string | undefined
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]!
    if (arg === "--session" || arg === "--kernel-url") {
      const value = argv[++index]
      if (!value) return USAGE
      if (arg === "--session") session = value
      else kernelUrl = value
    } else rest.push(arg)
  }
  if (!session) return USAGE
  return { session, ...(kernelUrl ? { kernelUrl } : {}), rest }
}

export async function runTerminalCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "terminal") return false
  const parsed = parseTerminalArgs(argv.slice(1))
  if (typeof parsed === "string") throw new Error(parsed)
  const command = parseRoomTerminalCommand(parsed.rest)
  if (typeof command === "string") throw new Error(command || ROOM_TERMINAL_USAGE)
  // Same endpoint rules as `chariox access`: this machine's kernel, with the local owner's credentials.
  const endpoint = parsed.kernelUrl
    ?? process.env.CHARIOX_KERNEL_URL
    ?? `ws://${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
  const client = new LocalIpcClient(endpoint)
  try {
    const resolved = await client.send<{ SessionResolved?: { session: RuntimeSession } }>(resolveSessionRequest(parsed.session))
    if (!resolved.SessionResolved) throw new Error(`No session "${parsed.session}". Check the id or alias.`)
    const message = await runRoomTerminalCommand({
      send: request => client.send(request),
      target: { sessionId: resolved.SessionResolved.session.id },
      input: process.stdin,
      output: process.stdout,
    }, command)
    process.stderr.write(`${message}\n`)
  } finally { await client.close() }
  return true
}
