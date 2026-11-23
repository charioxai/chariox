import { randomUUID } from "node:crypto"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { CommandFailure, familyUsageOf, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { defaultKernelEndpoint } from "./cli-options.js"
import { reportFailure, withKernel } from "./kernels-command.js"

const VERBS = ["list", "pause", "resume"] as const
type Verb = typeof VERBS[number]
type Send = (body: Record<string, unknown>) => Promise<unknown>
type Subscription = { subscription: { subscription_id: string }; paused: boolean }
type SubscriptionReply = { NotificationSubscriptions: { subscriptions: Subscription[] } }

function isVerb(value: string | undefined): value is Verb {
  return VERBS.some(verb => verb === value)
}

async function subscriptions(send: Send, verb: Verb, id?: string): Promise<unknown> {
  const paused = verb === "pause"
  const request = id ? {
    UpdateNotificationSubscription: {
      subscription_id: id,
      change: { paused: { paused } },
    },
  } : { ListNotificationSubscriptions: null }
  const { NotificationSubscriptions: reply } = await send(request) as SubscriptionReply
  if (!id) return reply.subscriptions
  const updated = reply.subscriptions.find(row => row.subscription.subscription_id === id)
  if (updated?.paused !== paused) throw new CommandFailure("notification_state_changed", `Subscription ${id} changed while it was being updated. Run chariox notifications list to read its current state.`)
  return { subscription_id: id, paused }
}

export async function runNotificationSubscriptionCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "notifications") return false
  let kernel: string | undefined
  try {
    const verb = argv[1]
    if (!isVerb(verb)) {
      const usage = familyUsageOf("notifications", VERBS)
      if (verb === "--help") { console.log(usage); return true }
      throw new CommandFailure("usage", usage)
    }
    const name = `notifications ${verb}`
    const rest = argv.slice(2)
    if (wantsHelp(rest)) { console.log(usageOf(name)); return true }
    const { flags, positionals } = parseArguments(name, rest)
    if (positionals.length !== (verb === "list" ? 0 : 1) || positionals[0]?.trim() === "") {
      throw new CommandFailure("usage", usageOf(name))
    }
    const [id] = positionals
    const socket = flags.get("--socket") as string | undefined
    kernel = flags.get("--kernel") as string | undefined
    const kernelUrl = flags.get("--kernel-url") as string | undefined
    const commandId = (flags.get("--command-id") as string | undefined) ?? (id ? randomUUID() : undefined)
    if (kernelUrl && (socket || kernel)) {
      throw new CommandFailure("usage", "Use --kernel-url for a direct terminal connection, or --socket and --kernel for your kernel access grant")
    }
    let reply: unknown
    if (kernelUrl || (!socket && !kernel && !process.env.CHARIOX_DAEMON_SOCKET)) {
      const client = new LocalIpcClient(kernelUrl ?? defaultKernelEndpoint(), { commandId, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 30_000 })
      try { reply = await subscriptions(body => client.send(body), verb, id) }
      finally { await client.close() }
    } else {
      reply = await withKernel({ socket, kernel, commandId, timeoutSeconds: 30 }, send => subscriptions(send, verb, id))
    }
    console.log(JSON.stringify(reply, null, 2))
  } catch (error) {
    reportFailure(error, kernel ?? null)
  }
  return true
}
