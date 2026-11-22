import assert from "node:assert/strict"
import test from "node:test"
import type { PaneCard, PaneFeed } from "@chariox/kernel-client/kernel-types"
import { initialPaneTitleState, paneTitleUpdate } from "./pane-title.js"

const card = (id: string): PaneCard => ({ id, asker_name: null, title: null, message: `Question ${id}`, choices: [], reply: null, text_terminal: true, requested_at_ms: 1, open_elsewhere: "Open in Chariox" })
const feed = (title: string, cards: readonly PaneCard[]): PaneFeed => ({
  title,
  identity: { room_id: "r", room_name: "Release", agent_id: "a", agent_name: "writer", parents: [], project_id: "p", placement: null },
  settings: { provider: "codex", account: null, model: null, effort: null, execution_mode: "build", permission_level: "required" },
  state: cards.length ? "waiting_for_answer" : "idle",
  goal: null,
  cards,
  stripes: [],
  notices: [],
  link: null,
})

test("the title stays on the waiting card until it is settled and is written only when it changes", () => {
  const waiting = paneTitleUpdate(initialPaneTitleState, feed("Release · writer · 1 card waiting", [card("q")]), false)
  assert.equal(waiting.output, "\u001b]0;Release · writer · 1 card waiting\u0007")
  assert.equal(paneTitleUpdate(waiting.state, feed("Release · writer · 1 card waiting", [card("q")]), false).output, "")
  assert.equal(paneTitleUpdate(waiting.state, feed("Release · writer", []), false).output, "\u001b]0;Release · writer\u0007")
})

test("a new card notifies the desktop only while the person is away, once", () => {
  const seen = paneTitleUpdate(initialPaneTitleState, feed("t", [card("old")]), false).state
  const away = paneTitleUpdate(seen, feed("t2", [card("old"), card("new")]), true)
  assert.equal(away.output, "\u001b]0;t2\u0007\u001b]9;writer: Question new\u0007")
  assert.equal(paneTitleUpdate(away.state, feed("t2", [card("old"), card("new")]), true).output, "")
})

test("no title text can end the escape sequence early", () => {
  assert.equal(paneTitleUpdate(initialPaneTitleState, feed("a\u0007b\u001b]0;x", []), false).output, "\u001b]0;a b ]0;x\u0007")
})
