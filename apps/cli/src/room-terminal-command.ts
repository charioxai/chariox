import {
  closeRoomTerminalRequest,
  expectRoomTerminalResult,
  listRoomTerminalsRequest,
  openRoomTerminalRequest,
  roomTerminalMinimumProtocolVersion,
  type RoomTerminalTarget,
} from "@chariox/kernel-client/ipc-requests"
import type { RoomTerminalSummary } from "@chariox/kernel-client/kernel-types"

import { withProtocolMinimum } from "./protocol-minimum-diagnostic.js"
import {
  attachRoomTerminal,
  type TerminalAttachInput,
  type TerminalAttachOutput,
  type TerminalSend,
} from "./room-terminal-attach.js"

export const ROOM_TERMINAL_USAGE =
  "usage: terminal [list | new [TITLE] | close TERMINAL | TERMINAL]  (Ctrl-] detaches; the terminal keeps running)"

export type RoomTerminalCommand =
  | { readonly kind: "list" }
  | { readonly kind: "new"; readonly title?: string }
  | { readonly kind: "close"; readonly selector: string }
  /** `selector` absent: the newest running terminal, else a new one. */
  | { readonly kind: "attach"; readonly selector?: string }

export function parseRoomTerminalCommand(args: readonly string[]): RoomTerminalCommand | string {
  const [first, ...rest] = args
  if (first === undefined) return { kind: "attach" }
  if (first === "list" && rest.length === 0) return { kind: "list" }
  if (first === "new") return rest.length ? { kind: "new", title: rest.join(" ") } : { kind: "new" }
  if (first === "close") return rest.length === 1 ? { kind: "close", selector: rest[0]! } : ROOM_TERMINAL_USAGE
  return args.length === 1 ? { kind: "attach", selector: first } : ROOM_TERMINAL_USAGE
}

/** A machine the list could not ask: the agent that runs there and what to do. Its terminals may exist and are not listed. */
export type UnavailableTerminals = { readonly host_agent: string; readonly message: string }

const notAsked = (unavailable: readonly UnavailableTerminals[]) => unavailable.map(machine => `${machine.host_agent}: ${machine.message}`).join("\n")

export function formatRoomTerminals(terminals: readonly RoomTerminalSummary[], unavailable: readonly UnavailableTerminals[] = []): string {
  if (terminals.length === 0 && unavailable.length === 0) return "No terminals in this Room yet. `terminal new` opens one."
  const lines = terminals.length === 0 ? "No terminals listed." : terminals.map((terminal, index) => {
    const state = terminal.status.state === "running"
      ? terminal.driver.state === "busy"
        ? `busy (driven by ${terminal.driver.driven_by.kind === "user" ? "you" : terminal.driver.driven_by.agent_id})`
        : "at prompt"
      : terminal.status.description
    const host = terminal.host_agent ? ` on ${terminal.host_agent}` : ""
    return `${index + 1}. ${terminal.terminal_id}  ${terminal.title}  ${state}${host}`
  }).join("\n")
  return unavailable.length === 0 ? lines : `${lines}\nNot listed (their terminals may still be running):\n${notAsked(unavailable)}`
}

async function listTerminals(send: TerminalSend, target: RoomTerminalTarget) {
  const reply = await withProtocolMinimum(() => send(listRoomTerminalsRequest(target)), {
    capability: "Room terminals",
    requestVariant: "RoomTerminal",
    minimumProtocolVersion: roomTerminalMinimumProtocolVersion,
  })
  const listed = expectRoomTerminalResult(reply, "listed")
  return { terminals: listed.terminals, unavailable: listed.unavailable ?? [] }
}

function pick(terminals: readonly RoomTerminalSummary[], selector: string, unavailable: readonly UnavailableTerminals[] = []): RoomTerminalSummary {
  const index = /^\d+$/.test(selector) ? terminals[Number(selector) - 1] : undefined
  const matches = index ? [index] : terminals.filter(terminal => terminal.terminal_id.startsWith(selector))
  if (matches.length === 1) return matches[0]!
  throw new Error(matches.length === 0
    ? `No terminal "${selector}" in this Room. Run \`terminal list\`.${unavailable.length ? `\nNot listed (it may be there):\n${notAsked(unavailable)}` : ""}`
    : `"${selector}" matches ${matches.length} terminals; use more of the id.`)
}

export type RoomTerminalCommandDeps = {
  send: TerminalSend
  target: RoomTerminalTarget
  input: TerminalAttachInput
  output: TerminalAttachOutput
  /** Run `attach` with the outer terminal handed over (the TUI suspends its renderer). */
  withScreen?<T>(attach: () => Promise<T>): Promise<T>
  signal?: AbortSignal
}

/** Run one `terminal` command; the returned text is what to tell the user afterwards. */
export async function runRoomTerminalCommand(deps: RoomTerminalCommandDeps, command: RoomTerminalCommand): Promise<string> {
  const { send, target, output } = deps
  const size = { cols: output.columns || 80, rows: output.rows || 24 }
  if (command.kind === "list") {
    const listed = await listTerminals(send, target)
    return formatRoomTerminals(listed.terminals, listed.unavailable)
  }
  if (command.kind === "close") {
    const listed = await listTerminals(send, target)
    const terminal = pick(listed.terminals, command.selector, listed.unavailable)
    expectRoomTerminalResult(await send(closeRoomTerminalRequest({ ...target, ...(terminal.host_agent ? { hostAgent: terminal.host_agent } : {}) }, terminal.terminal_id)), "closed")
    return `Closed terminal ${terminal.terminal_id}.`
  }
  const { terminals, unavailable } = await listTerminals(send, target)
  let terminal: RoomTerminalSummary
  if (command.kind === "attach" && command.selector) {
    terminal = pick(terminals, command.selector, unavailable)
  } else if (command.kind === "attach" && unavailable.length > 0 && !terminals.some(candidate => candidate.status.state === "running")) {
    // The terminal that is there may be on a machine the list could not ask: no shell opens on a guess.
    return `Terminals on some machines could not be listed:\n${notAsked(unavailable)}\n\`terminal new\` opens one on this kernel.`
  } else {
    const running = command.kind === "attach" ? terminals.filter(candidate => candidate.status.state === "running").at(-1) : undefined
    terminal = running ?? expectRoomTerminalResult(
      await send(openRoomTerminalRequest(target, size, command.kind === "new" && command.title ? { title: command.title } : {})),
      "opened",
    ).terminal
  }
  if (terminal.status.state === "ended") return `Terminal ${terminal.terminal_id} has ${terminal.status.description}. \`terminal new\` opens another.`
  const attach = () => attachRoomTerminal({
    send, target, terminal, input: deps.input, output, ...(deps.signal ? { signal: deps.signal } : {}),
  })
  const end = await (deps.withScreen ? deps.withScreen(attach) : attach())
  return end.reason === "ended"
    ? `Terminal ${terminal.terminal_id} ${end.description}.`
    : `Detached from terminal ${terminal.terminal_id}; it keeps running. \`terminal ${terminal.terminal_id}\` re-attaches.`
}
