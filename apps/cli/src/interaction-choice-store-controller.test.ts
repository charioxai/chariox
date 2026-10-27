import assert from "node:assert/strict"
import test from "node:test"

import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"

test("interaction choice store tracks selected choices with renderer fallback", () => {
  const store = createInteractionChoiceStoreController()

  assert.equal(store.getSelectedIndex("interaction-a"), undefined)
  assert.equal(store.selectedChoiceIndex("interaction-a"), 0)

  store.setSelectedIndex("interaction-a", 2)
  assert.equal(store.getSelectedIndex("interaction-a"), 2)
  assert.equal(store.selectedChoiceIndex("interaction-a"), 2)
  assert.equal(store.selectedChoiceIndex("interaction-b"), 0)
})

test("interaction choice store tracks custom replies and editing state", () => {
  const store = createInteractionChoiceStoreController()

  assert.equal(store.customReply("interaction-a"), "")
  assert.equal(store.getStoredCustomReply("interaction-a"), undefined)
  assert.equal(store.isCustomEditing("interaction-a"), false)

  store.setCustomReply("interaction-a", "custom reply")
  store.setCustomEditing("interaction-a", true)
  assert.equal(store.customReply("interaction-a"), "custom reply")
  assert.equal(store.getStoredCustomReply("interaction-a"), "custom reply")
  assert.equal(store.isCustomEditing("interaction-a"), true)

  store.clearCustomReply("interaction-a")
  store.setCustomEditing("interaction-a", false)
  assert.equal(store.customReply("interaction-a"), "")
  assert.equal(store.getStoredCustomReply("interaction-a"), undefined)
  assert.equal(store.isCustomEditing("interaction-a"), false)
})
