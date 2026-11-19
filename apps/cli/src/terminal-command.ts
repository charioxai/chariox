import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { resolveSessionRequest, type RoomTerminalTarget } from "@chariox/kernel-client/ipc-requests"
import type { RuntimeSession } from "@chariox/kernel-client/kernel-types"

import { TERMINAL_ATTACH_USAGE } from "./terminal-verbs.js"
import { withKernel } from "./kernels-command.js"
import type { SendRequest } from "./kernel-files.js"
import type { TerminalSend } from "./room-terminal-attach.js"
import { parseRoomTerminalCommand, ROOM_TERMINAL_USAGE, runRoomTerminalCommand } from "./room-terminal-command.js"

const USAGE = TERMINAL_ATTACH_USAGE

export function parseTerminalArgs(argv: readonly string[]): { session?: string; kernel?: string; kernelUrl?: string; rest: string[] } | string {
  const rest: string[] = []
  let session: string | undefined
  let kernelUrl: string | undefined
  let kernel: string | undefined
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]!
    if (arg === "--session" || arg === "--kernel-url" || arg === "--kernel") {
      const value = argv[++index]
      if (!value) return USAGE
      if (arg === "--session") session = value
      else if (arg === "--kernel") kernel = value
      else kernelUrl = value
    } else rest.push(arg)
  }
  if (kernel && kernelUrl) return USAGE
  return { ...(session ? { session } : {}), ...(kernel ? { kernel } : {}), ...(kernelUrl ? { kernelUrl } : {}), rest }
}

export async function runTerminalCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "terminal") return false
  const parsed = parseTerminalArgs(argv.slice(1))
  if (typeof parsed === "string") throw new Error(parsed)
  const command = parseRoomTerminalCommand(parsed.rest)
  if (typeof command === "string") throw new Error(command || ROOM_TERMINAL_USAGE)
  // Without a Room the terminal is the kernel's own.
  const attach = async (sendKernel: SendRequest) => {
    const send: TerminalSend = request => sendKernel(request as Record<string, unknown>)
    let target: RoomTerminalTarget = { kernelTerminal: true }
    if (parsed.session) {
      const resolved = await send(resolveSessionRequest(parsed.session)) as { SessionResolved?: { session: RuntimeSession } }
      if (!resolved.SessionResolved) throw new Error(`No session "${parsed.session}". Check the id or alias.`)
      target = { sessionId: resolved.SessionResolved.session.id }
    }
    return runRoomTerminalCommand({ send, target, input: process.stdin, output: process.stdout }, command)
  }
  // --kernel NAME reaches another of the owner's kernels through this machine's kernel and its access grant.
  if (parsed.kernel) {
    process.stderr.write(`${await withKernel({ socket: undefined, kernel: parsed.kernel, timeoutSeconds: 240 + 30 }, attach)}\n`)
    return true
  }
  // Same endpoint rules as `chariox access`: this machine's kernel, with the local owner's credentials.
  const endpoint = parsed.kernelUrl
    ?? process.env.CHARIOX_KERNEL_URL
    ?? `ws://${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
  const client = new LocalIpcClient(endpoint)
  try {
    process.stderr.write(`${await attach(request => client.send(request))}\n`)
  } finally { await client.close() }
  return true
}
