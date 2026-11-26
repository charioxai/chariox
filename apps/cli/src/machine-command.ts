import type { OwnerInteraction, OwnerInteractionsListedResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseArguments, usageOf } from "./agent-command-table.js"
import { reportFailure, withKernel } from "./kernels-command.js"

// An acquire may wait for the owner's decision (the kernel waits up to 300 s) before the service answers.
const ACQUIRE_SECONDS = 330
const REQUEST_SECONDS = 60
const CARD_POLL_MS = 1500
const USAGE = ["machine options", "machine acquire", "machine status", "machine release", "machine limits", "machine set-limits"].map(usageOf).join(" | ")

type Send = (body: Record<string, unknown>) => Promise<unknown>
type Flags = Map<string, string | true>

function timeoutOf(flags: Flags, fallback: number): number {
  const value = flags.get("--timeout")
  if (value === undefined) return fallback
  const seconds = Number(value)
  if (!Number.isFinite(seconds) || seconds <= 0) throw new CommandFailure("usage", "--timeout needs a positive number of seconds")
  return seconds
}

async function acquisition(flags: Flags, fallbackSeconds: number, request: Record<string, unknown>, use: (send: Send) => Promise<unknown> = send => send({ MachineAcquisition: request })): Promise<void> {
  const result = await withKernel({ socket: flags.get("--socket") as string | undefined, kernel: flags.get("--kernel") as string | undefined, timeoutSeconds: timeoutOf(flags, fallbackSeconds) }, use)
  console.log(JSON.stringify((result as { MachineAcquisition: { result: unknown } }).MachineAcquisition.result, null, 2))
}

const MACHINE_CARD = "machine-acquisition-"
const LIMITS_CARD = "limits-raise-"

async function pendingCard(send: Send, prefix: string): Promise<OwnerInteraction | undefined> {
  const listed = (await send({ ListOwnerInteractions: {} })) as OwnerInteractionsListedResponse
  return listed.OwnerInteractionsListed.interactions.find(card => card.interaction_id.startsWith(prefix))
}

// An agent acting for the owner gets the owner's card; say so once while the kernel waits for the answer.
async function reportPendingCard(send: Send, answered: Promise<unknown>, prefix = MACHINE_CARD, what = "this machine"): Promise<void> {
  let settled = false
  void answered.then(() => { settled = true }, () => { settled = true })
  while (!settled) {
    await new Promise(resolve => setTimeout(resolve, CARD_POLL_MS))
    if (settled) return
    const card = await pendingCard(send, prefix).catch(() => undefined)
    if (card) {
      console.error(`Waiting for the owner to approve ${what} (card ${card.interaction_id}).`)
      return
    }
  }
}

async function options(argv: string[]): Promise<void> {
  const { flags } = parseArguments("machine options", argv)
  await acquisition(flags, REQUEST_SECONDS, { action: "options" })
}

async function acquire(argv: string[]): Promise<void> {
  const { flags, positionals: [classId] } = parseArguments("machine acquire", argv)
  const region = flags.get("--region")
  const requestId = flags.get("--request-id")
  const request = { action: "acquire", class_id: classId, ...(typeof region === "string" ? { region } : {}), ...(typeof requestId === "string" ? { request_id: requestId } : {}) }
  await acquisition(flags, ACQUIRE_SECONDS, request, async send => {
    const answered = send({ MachineAcquisition: request })
    const [result] = await Promise.all([answered, reportPendingCard(send, answered)])
    return result
  })
}

async function status(argv: string[]): Promise<void> {
  const { flags, positionals: [allocationRef] } = parseArguments("machine status", argv)
  await acquisition(flags, REQUEST_SECONDS, { action: "status", allocation_ref: allocationRef })
}

async function release(argv: string[]): Promise<void> {
  const { flags, positionals: [allocationRef] } = parseArguments("machine release", argv)
  await acquisition(flags, REQUEST_SECONDS, { action: "release", allocation_ref: allocationRef })
}

async function limits(argv: string[]): Promise<void> {
  const { flags } = parseArguments("machine limits", argv)
  await acquisition(flags, REQUEST_SECONDS, { action: "limits" })
}

// A decimal amount in the account's currency, as millionths.
function microsOf(text: string): number {
  if (!/^\d{1,9}(\.\d{1,6})?$/.test(text)) throw new CommandFailure("usage", "--spend needs an amount such as 15 or 7.50")
  const [whole, fraction = ""] = text.split(".")
  return Number(whole) * 1_000_000 + Number(fraction.padEnd(6, "0"))
}

function countOf(text: string): number {
  const count = Number(text)
  if (!Number.isInteger(count) || count < 1) throw new CommandFailure("usage", "--machines needs a whole number of machines, 1 or more")
  return count
}

async function setLimits(argv: string[]): Promise<void> {
  const { flags } = parseArguments("machine set-limits", argv)
  const scope = flags.get("--scope") ?? "account"
  if (scope !== "account" && scope !== "no-ask") throw new CommandFailure("usage", "--scope is account or no-ask")
  const machines = flags.get("--machines")
  const spend = flags.get("--spend")
  if (typeof machines !== "string" && typeof spend !== "string") throw new CommandFailure("usage", "name --machines or --spend to change")
  const request = {
    action: "set_limits", scope: scope === "no-ask" ? "no_ask" : "account",
    ...(typeof machines === "string" ? { machines_at_once: countOf(machines) } : {}),
    ...(typeof spend === "string" ? { spend_per_day_micros: microsOf(spend) } : {}),
  }
  await acquisition(flags, ACQUIRE_SECONDS, request, async send => {
    const answered = send({ MachineAcquisition: request })
    const [result] = await Promise.all([answered, reportPendingCard(send, answered, LIMITS_CARD, "this change")])
    return result
  })
}

export async function runMachineCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "machine") return false
  try {
    if (argv[1] === "--help") console.log(USAGE)
    else if (argv[1] === "options") await options(argv.slice(2))
    else if (argv[1] === "acquire") await acquire(argv.slice(2))
    else if (argv[1] === "status") await status(argv.slice(2))
    else if (argv[1] === "release") await release(argv.slice(2))
    else if (argv[1] === "limits") await limits(argv.slice(2))
    else if (argv[1] === "set-limits") await setLimits(argv.slice(2))
    else throw new CommandFailure("usage", USAGE)
  } catch (error) { reportFailure(error) }
  return true
}
