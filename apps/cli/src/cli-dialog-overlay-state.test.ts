import assert from "node:assert/strict"
import test from "node:test"

import {
  cliDialogOverlayIsOpen,
  resolveCliDialogOverlayMode,
} from "./cli-dialog-overlay-state.js"

test("resolveCliDialogOverlayMode follows overlay render priority", () => {
  assert.equal(resolveCliDialogOverlayMode({
    hotkeysOpen: false,
    terminalPairingOpen: false,
    sessionBrowserOpen: false,
  }), "closed")
  assert.equal(resolveCliDialogOverlayMode({
    hotkeysOpen: true,
    terminalPairingOpen: false,
    sessionBrowserOpen: false,
  }), "hotkeys")
  assert.equal(resolveCliDialogOverlayMode({
    hotkeysOpen: true,
    terminalPairingOpen: true,
    sessionBrowserOpen: false,
  }), "terminal-pairing")
  assert.equal(resolveCliDialogOverlayMode({
    hotkeysOpen: true,
    terminalPairingOpen: true,
    sessionBrowserOpen: true,
  }), "session-browser")
})

test("cliDialogOverlayIsOpen detects any active dialog", () => {
  assert.equal(cliDialogOverlayIsOpen({
    hotkeysOpen: false,
    terminalPairingOpen: false,
    sessionBrowserOpen: false,
  }), false)
  assert.equal(cliDialogOverlayIsOpen({
    hotkeysOpen: false,
    terminalPairingOpen: true,
    sessionBrowserOpen: false,
  }), true)
})
