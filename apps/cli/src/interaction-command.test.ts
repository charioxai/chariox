// An external agent answers the person's cards from a shell, through the kernel's own requests.
import assert from "node:assert/strict"
import { test } from "node:test"
import { runInteractionCommand } from "./interaction-command.js"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"

const card = { session_id: "room-1", interaction_id: "task-blocked-1", reply_choice_id: "reply", reply_choice_description: "Send your reply and continue the waiting task.", title: "Allow browser access", message: "m", choices: [{ id: "cancel", label: "Dismiss", description: "Hide the card. The task keeps waiting." }, { id: "allow_browser", label: "Allow browser" }] }
const detached = { interaction_id: "app_host_1", title: "Open a link from an App", message: "m", choices: [{ id: "decline", label: "Decline" }] }
const listed = { response: { OwnerInteractionsListed: { interactions: [card, detached] } } }

async function run(argv: string[]) {
  const kernel = await startFakeKernel((socket, frame) =>
    replyWith(socket, frame, "ListOwnerInteractions" in frame.request ? listed : { response: { InteractionResponded: { interaction_id: "task-blocked-1" } } }))
  const previous = { out: console.log, exit: process.exitCode }
  let out = ""
  let err = ""
  const write = process.stderr.write
  console.log = (line: string) => { out += line }
  process.stderr.write = ((chunk: string) => { err += chunk; return true }) as typeof process.stderr.write
  try {
    await runInteractionCommand([...argv, "--socket", kernel.path])
    return { out, err, exit: process.exitCode as number | undefined, requests: kernel.frames.map(frame => frame.request) }
  } finally {
    console.log = previous.out
    process.stderr.write = write
    process.exitCode = previous.exit
    await kernel.close()
  }
}

test("interaction list prints the owner's open cards", async () => {
  const result = await run(["interaction", "list"])
  assert.deepEqual(result.requests, [{ ListOwnerInteractions: {} }])
  assert.deepEqual(JSON.parse(result.out), [card, detached])
})

test("interaction answer finds the card's Room and answers it", async () => {
  const result = await run(["interaction", "answer", "task-blocked-1", "allow_browser"])
  assert.deepEqual(result.requests, [
    { ListOwnerInteractions: {} },
    { RespondToInteraction: { session_id: "room-1", interaction_id: "task-blocked-1", choice_id: "allow_browser" } },
  ])
  assert.deepEqual(JSON.parse(result.out), { interaction_id: "task-blocked-1", choice_id: "allow_browser", status: "answered" })
  const reply = await run(["interaction", "answer", "task-blocked-1", "reply", "--reply", "go on"])
  assert.match(reply.err, /Send your reply and continue the waiting task/)
  assert.deepEqual(reply.requests[1], { RespondToInteraction: { session_id: "room-1", interaction_id: "task-blocked-1", choice_id: "reply", custom_reply: "go on" } })
})

test("interaction answer settles a card outside any Room with its own request", async () => {
  const result = await run(["interaction", "answer", "app_host_1", "decline"])
  assert.deepEqual(result.requests, [
    { ListOwnerInteractions: {} },
    { AnswerUserDomainInteraction: { interaction_id: "app_host_1", choice_id: "decline" } },
  ])
  assert.deepEqual(JSON.parse(result.out), { interaction_id: "app_host_1", choice_id: "decline", status: "answered" })
})

test("interaction answer names a card that is not open and sends no answer", async () => {
  const result = await run(["interaction", "answer", "gone-1", "allow"])
  assert.deepEqual(result.requests, [{ ListOwnerInteractions: {} }])
  assert.match(result.err, /no open card gone-1/)
  assert.ok(result.exit)
})

test("interaction answer describes Dismiss before sending the existing choice id", async () => {
  const result = await run(["interaction", "answer", "task-blocked-1", "cancel"])
  assert.match(result.err, /Dismiss\. Hide the card\. The task keeps waiting\./)
  assert.deepEqual(result.requests[1], { RespondToInteraction: { session_id: "room-1", interaction_id: "task-blocked-1", choice_id: "cancel" } })
})
