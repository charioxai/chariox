// What a provider terminal shows outside its screen grid: the terminal title, which stays while
// a card waits (Codex has no status line), and a desktop notification for each new card while
// the person is away. Both are written by the launcher, never drawn over the provider's screen.
import type { PaneCard, PaneFeed } from "@chariox/kernel-client/kernel-types"

const ESC = "\u001b"
const BEL = "\u0007"

/** A title or notification text carries no control characters that could end the sequence. */
function printable(text: string): string {
  return text.replace(/[\u0000-\u001f\u007f-\u009f]/g, " ").trim()
}

/** OSC 0: the kernel's title line as the terminal's window and tab title. */
export function terminalTitleSequence(feed: PaneFeed): string {
  return `${ESC}]0;${printable(feed.title)}${BEL}`
}

/** OSC 9: a desktop notification naming who asks and what. */
export function cardNotificationSequence(feed: PaneFeed, card: PaneCard): string {
  const asker = card.asker_name ?? feed.identity.agent_name
  return `${ESC}]9;${printable(`${asker}: ${card.title ?? card.message}`)}${BEL}`
}

export type PaneTitleState = { readonly title: string | null; readonly seenCards: ReadonlySet<string> }

export const initialPaneTitleState: PaneTitleState = { title: null, seenCards: new Set() }

/**
 * The bytes to write for one feed update: the title when it changed, and one notification per
 * card the launcher has not seen while the person is away from the terminal.
 */
export function paneTitleUpdate(state: PaneTitleState, feed: PaneFeed, away: boolean): { readonly output: string; readonly state: PaneTitleState } {
  let output = state.title === feed.title ? "" : terminalTitleSequence(feed)
  if (away) for (const card of feed.cards) if (!state.seenCards.has(card.id)) output += cardNotificationSequence(feed, card)
  return { output, state: { title: feed.title, seenCards: new Set(feed.cards.map(card => card.id)) } }
}
