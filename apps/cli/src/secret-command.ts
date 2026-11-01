import type { VaultSecretRequestedResponse, VaultSecretStatus, VaultSecretStatusResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseArguments, usageOf } from "./agent-command-table.js"
import { reportFailure, withKernel } from "./kernels-command.js"

const POLL_MS = 2000
const REQUEST_SECONDS = 30
const WAIT_SECONDS = 660
const SETTLED_EXIT: Record<Exclude<VaultSecretStatus, "pending">, number> = { stored: 0, cancelled: 3, timed_out: 5 }

type Send = (body: Record<string, unknown>) => Promise<unknown>

function seconds(value: string | true | undefined): number {
  if (value === undefined) return WAIT_SECONDS
  const parsed = Number(value)
  if (!Number.isFinite(parsed) || parsed <= 0) throw new CommandFailure("usage", "--timeout needs a positive number of seconds")
  return parsed
}

async function status(send: Send, interactionId: string): Promise<VaultSecretStatusResponse["VaultSecretStatusReported"]> {
  return ((await send({ GetVaultSecretStatus: { interaction_id: interactionId } })) as VaultSecretStatusResponse).VaultSecretStatusReported
}

async function request(argv: string[]): Promise<number> {
  const { flags, positionals } = parseArguments("secret request", argv)
  const label = flags.get("--label")
  if (typeof label !== "string" || !label.trim()) throw new CommandFailure("usage", usageOf("secret request"))
  const socket = flags.get("--socket") as string | undefined
  const waitSeconds = seconds(flags.get("--timeout"))
  return withKernel({ socket, timeoutSeconds: REQUEST_SECONDS }, async send => {
    const asked = await send({ RequestVaultSecret: { credential_id: positionals[0], label, overwrite: flags.has("--overwrite") } })
    const requested = (asked as VaultSecretRequestedResponse).VaultSecretRequested
    console.log(requested.link)
    console.log(JSON.stringify(requested))
    if (flags.has("--no-wait")) return 6
    const deadline = Date.now() + waitSeconds * 1000
    for (;;) {
      const current = await status(send, requested.interaction_id)
      if (current.status !== "pending") {
        console.log(JSON.stringify({ interaction_id: current.interaction_id, credential_id: current.credential_id, status: current.status }))
        return SETTLED_EXIT[current.status]
      }
      if (Date.now() >= deadline) throw new CommandFailure("kernel_request_timeout", `the user has not answered yet; run chariox secret status ${requested.interaction_id}`)
      await new Promise(resolve => setTimeout(resolve, POLL_MS))
    }
  })
}

async function statusOf(argv: string[]): Promise<number> {
  const { flags, positionals } = parseArguments("secret status", argv)
  const current = await withKernel({ socket: flags.get("--socket") as string | undefined, timeoutSeconds: REQUEST_SECONDS }, send => status(send, positionals[0]!))
  console.log(JSON.stringify(current))
  return current.status === "pending" ? 6 : SETTLED_EXIT[current.status]
}

export async function runSecretCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "secret") return false
  try {
    if (argv[1] === "request") process.exitCode = await request(argv.slice(2))
    else if (argv[1] === "status") process.exitCode = await statusOf(argv.slice(2))
    else throw new CommandFailure("usage", `${usageOf("secret request")} | ${usageOf("secret status")}`)
  } catch (error) { reportFailure(error) }
  return true
}
