import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { arrobaArtFrame, createWaitingRoomState, cycleWaitingRoomValue, moveWaitingRoomFocus, waitingRoomRows } from "./waiting-room.js"

test("waiting room cycles model and effort from provider catalog", () => {
  const catalog = {
    ...fallbackProviderCatalog(),
    all: [
      {
        id: "openai",
        name: "OpenAI",
        models: {
          "gpt-5.4": { id: "gpt-5.4", name: "GPT-5.4", status: "active", variants: { low: {}, high: {} } },
          "gpt-5-mini": { id: "gpt-5-mini", name: "GPT-5 mini", status: "active", variants: { low: {} } },
        },
      },
    ],
    default: { openai: "gpt-5.4" },
  }
  let state = createWaitingRoomState([], catalog, "openai/gpt-5.4", "high")
  state = moveWaitingRoomFocus(state, 2)
  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(waitingRoomRows(state, [], catalog)[2]?.value, "OpenAI GPT-5 mini")
  state = moveWaitingRoomFocus(state, 1)
  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(waitingRoomRows(state, [], catalog)[3]?.value, "Low")
})

test("arrobaArtFrame resolves to the clean logo after the intro completes", () => {
  const first = arrobaArtFrame(0)
  const last = arrobaArtFrame(12)
  assert.notEqual(first, last)
  assert.equal(last.includes("____"), true)
})
