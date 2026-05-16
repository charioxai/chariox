import assert from "node:assert/strict"
import test from "node:test"

import {
  nextSessionBrowserIndex,
  resolveSessionBrowserKeyAction,
} from "./session-browser-key-policy.js"

test("resolveSessionBrowserKeyAction ignores inactive or modified events", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: false,
    event: { name: "enter" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter", eventType: "release" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter", ctrl: true },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
})

test("resolveSessionBrowserKeyAction handles close and movement keys", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "escape" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "close" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "up" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "move", delta: -1 })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "down" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "move", delta: 1 })
})

test("resolveSessionBrowserKeyAction handles empty, submit, and lifecycle keys", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter" },
    sessionCount: 0,
    selectedIndex: 0,
  }), { action: "empty" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "submit", selectedIndex: 1 })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "a" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "lifecycle", selectedIndex: 1, lifecycleAction: "archive" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "delete" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "lifecycle", selectedIndex: 1, lifecycleAction: "delete" })
})

test("nextSessionBrowserIndex wraps across available sessions", () => {
  assert.equal(nextSessionBrowserIndex(0, -1, 3), 2)
  assert.equal(nextSessionBrowserIndex(2, 1, 3), 0)
  assert.equal(nextSessionBrowserIndex(2, 1, 0), 2)
})
