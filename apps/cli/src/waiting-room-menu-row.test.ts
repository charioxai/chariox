import assert from "node:assert/strict"
import test from "node:test"

import { formatWaitingRoomMenuRow } from "./waiting-room-menu-row.js"
import type { WaitingRoomRow } from "./waiting-room-types.js"

test("formatWaitingRoomMenuRow bounds long waiting room rows at 80 columns", () => {
  const row = waitingRoomRow({
    title: "* session-working (frontend)",
    columns: [
      "Working",
      "home-kernel@home-machine",
      "managed",
      "1 working, 1 active prompt, 2 queued prompts",
      "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent",
      "2026-01-02 10:00 UTC",
      "2026-01-01 09:00 UTC",
    ],
  })

  const rendered = formatWaitingRoomMenuRow(row, 80)

  assert.equal(rendered.length, 80)
  assert.equal(rendered.endsWith("..."), true)
  assert.match(rendered, /^>   \* session-working/)
})

test("formatWaitingRoomMenuRow preserves wider waiting room rows", () => {
  const row = waitingRoomRow({
    title: "session-done",
    columns: [
      "Done",
      "local",
      "off",
      "-",
      "open session",
      "2026-01-02 10:00 UTC",
      "2026-01-01 09:00 UTC",
    ],
  })

  const rendered = formatWaitingRoomMenuRow(row, 220)

  assert.equal(rendered.includes("open session"), true)
  assert.equal(rendered.endsWith("..."), false)
  assert.equal(rendered.length < 220, true)
})

function waitingRoomRow(overrides: Partial<WaitingRoomRow> = {}): WaitingRoomRow {
  return {
    id: "session:session-1",
    title: "session-1",
    value: "Active",
    titleWidth: 32,
    indent: 1,
    focused: true,
    selectable: true,
    scrollbar: "",
    ...overrides,
  }
}
