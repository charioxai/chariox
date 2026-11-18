import type { OwnerInteraction, OwnerInteractionsListedResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseArguments, usageOf } from "./agent-command-table.js"
import { reportFailure, withKernel } from "./kernels-command.js"

const REQUEST_SECONDS = 30
const USAGE = `${usageOf("interaction list")} | ${usageOf("interaction answer")}`

type Send = (body: Record<string, unknown>) => Promise<unknown>

async function ownerCards(send: Send): Promise<OwnerInteraction[]> {
  return ((await send({ ListOwnerInteractions: {} })) as OwnerInteractionsListedResponse).OwnerInteractionsListed.interactions
}

async function list(argv: string[]): Promise<void> {
  const { flags, positionals } = parseArguments("interaction list", argv)
  if (positionals.length) throw new CommandFailure("usage", usageOf("interaction list"))
  const cards = await withKernel({ socket: flags.get("--socket") as string | undefined, kernel: flags.get("--kernel") as string | undefined, timeoutSeconds: REQUEST_SECONDS }, ownerCards)
  console.log(JSON.stringify(cards, null, 2))
}

// The card's Room comes from the person's list, so the agent names only the card and the choice.
async function answer(argv: string[]): Promise<void> {
  const { flags, positionals } = parseArguments("interaction answer", argv)
  const [id, choice] = positionals
  if (!id || !choice || positionals.length !== 2) throw new CommandFailure("usage", usageOf("interaction answer"))
  const reply = flags.get("--reply")
  await withKernel({ socket: flags.get("--socket") as string | undefined, kernel: flags.get("--kernel") as string | undefined, timeoutSeconds: REQUEST_SECONDS }, async send => {
    const card = (await ownerCards(send)).find(entry => entry.interaction_id === id)
    if (!card) throw new CommandFailure("usage", `no open card ${id}; run chariox interaction list`)
    await send({ RespondToInteraction: { session_id: card.session_id, interaction_id: id, choice_id: choice, ...(typeof reply === "string" ? { custom_reply: reply } : {}) } })
    console.log(JSON.stringify({ interaction_id: id, choice_id: choice, status: "answered" }))
  })
}

export async function runInteractionCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "interaction") return false
  try {
    if (argv[1] === "--help") console.log(USAGE)
    else if (argv[1] === "list") await list(argv.slice(2))
    else if (argv[1] === "answer") await answer(argv.slice(2))
    else throw new CommandFailure("usage", USAGE)
  } catch (error) { reportFailure(error) }
  return true
}
