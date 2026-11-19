// What the ack-drop drill selects and reads back: the lifecycle kinds it arms and how it finds the wake in history.
// An events_timer fires through Operation::FireWakes, whose delivery kind is wake_intent (agent_lifecycle/faults.rs).
export const DEFAULT_KINDS = ["wake_intent", "delivery_receipt"]

const promptText = turn => `${turn.user_prompt?.entry?.text ?? ""}`

/** The turns the kernel started on its own for a wake. Their prompt carries event counts and kinds, never the timer's label. */
export const wakeTurns = turns => turns.filter(turn => turn.system_prompt === true && promptText(turn).startsWith("Inbox:"))

/** The agent's answer to the wake: provider output that is exactly the requested reply. The kernel's outline puts a turn's final reply in `summary`, the rest in `entries`. */
export const wokeReply = (turns, reply) => turns.some(turn => [turn.summary, ...(turn.entries ?? [])].some(item => item?.entry?.kind === "provider_output" && item.entry.text.trim() === reply))
