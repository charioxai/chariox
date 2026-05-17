import assert from "node:assert/strict"
import test from "node:test"

import { createCliDialogOverlayController } from "./cli-dialog-overlay-controller.js"
import type { CliDialogFocusSnapshot, CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"
import type { CliDialogOverlayMode } from "./cli-dialog-overlay.js"
import type { TerminalPairingLinkView } from "./relay-api.js"

test("dialog overlay controller opens and closes hotkeys with focus restore", () => {
  const harness = createHarness()
  const controller = createCliDialogOverlayController(harness.deps)

  controller.openHotkeys()

  assert.equal(harness.hotkeysOpen(), true)
  assert.equal(harness.currentFocus.blurCount, 1)
  assert.equal(controller.savedFocusDebug()?.id, "current")
  assert.deepEqual(harness.renderModes(), ["hotkeys"])

  controller.closeHotkeys()
  assert.equal(harness.hotkeysOpen(), false)
  assert.deepEqual(harness.renderModes(), ["hotkeys", "closed"])
  assert.equal(harness.currentFocus.focusCount, 0)

  harness.flushFocusRestores()
  assert.equal(harness.currentFocus.focusCount, 1)
  assert.equal(controller.savedFocusDebug(), null)
})

test("dialog overlay controller creates terminal pairing links", async () => {
  const harness = createHarness()
  const controller = createCliDialogOverlayController(harness.deps)

  await controller.openTerminalPairing()

  assert.equal(harness.terminalPairingOpen(), true)
  assert.equal(harness.terminalPairing()?.pairing_code, "123456")
  assert.deepEqual(harness.terminalPairingQrLines(), ["qr"])
  assert.deepEqual(harness.renderModes(), ["terminal-pairing", "terminal-pairing"])
  assert.equal(harness.footerMessages().at(-1)?.message, "terminal pairing link created")
})

test("dialog overlay controller closes terminal pairing on creation failure", async () => {
  const harness = createHarness({
    createTerminalPairingLink: async () => {
      throw new Error("pairing unavailable")
    },
  })
  const controller = createCliDialogOverlayController(harness.deps)

  await controller.openTerminalPairing()

  assert.equal(harness.terminalPairingOpen(), false)
  assert.deepEqual(harness.renderModes(), ["terminal-pairing", "closed"])
  assert.equal(harness.footerMessages().at(-1)?.message, "pairing unavailable")
})

test("dialog overlay controller opens session browser only when sessions exist", () => {
  const emptyHarness = createHarness({ sessionCount: 0 })
  createCliDialogOverlayController(emptyHarness.deps).openSessionBrowser()

  assert.equal(emptyHarness.sessionBrowserOpen(), false)
  assert.equal(emptyHarness.footerMessages().at(-1)?.message, "no sessions available to join")
  assert.deepEqual(emptyHarness.renderModes(), [])

  const harness = createHarness({
    hotkeysOpen: true,
    terminalPairingOpen: true,
    sessionCount: 4,
    waitingRoomSessionIndex: 99,
  })
  createCliDialogOverlayController(harness.deps).openSessionBrowser()

  assert.equal(harness.hotkeysOpen(), false)
  assert.equal(harness.terminalPairingOpen(), false)
  assert.equal(harness.sessionBrowserOpen(), true)
  assert.equal(harness.sessionBrowserIndex(), 3)
  assert.deepEqual(harness.renderModes(), ["session-browser"])
})

test("dialog overlay controller closes the highest-priority active dialog", () => {
  const harness = createHarness({
    hotkeysOpen: true,
    terminalPairingOpen: true,
    sessionBrowserOpen: true,
  })
  const controller = createCliDialogOverlayController(harness.deps)

  assert.equal(controller.isOpen(), true)
  controller.closeActive()

  assert.equal(harness.sessionBrowserOpen(), false)
  assert.equal(harness.terminalPairingOpen(), true)
  assert.equal(harness.hotkeysOpen(), true)
  assert.deepEqual(harness.renderModes(), ["terminal-pairing"])
})

function createHarness(options: {
  hotkeysOpen?: boolean
  terminalPairingOpen?: boolean
  sessionBrowserOpen?: boolean
  sessionCount?: number
  waitingRoomSessionIndex?: number
  createTerminalPairingLink?: () => Promise<TerminalPairingLinkView>
} = {}) {
  let hotkeysOpen = options.hotkeysOpen ?? false
  let terminalPairingOpen = options.terminalPairingOpen ?? false
  let sessionBrowserOpen = options.sessionBrowserOpen ?? false
  let terminalPairing: TerminalPairingLinkView | null = null
  let terminalPairingQrLines: string[] = []
  let sessionBrowserIndex = 0
  const renderModes: CliDialogOverlayMode[] = []
  const focusRestores: Array<() => void> = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const currentFocus = new MockFocusTarget("current")
  const promptFocus = new MockFocusTarget("prompt")

  const deps = {
    getOpenState: () => ({ hotkeysOpen, terminalPairingOpen, sessionBrowserOpen }),
    getCurrentFocus: () => currentFocus,
    getPromptFocus: () => promptFocus,
    describeFocus: (target: MockFocusTarget | null | undefined): CliDialogFocusSnapshot | null => target
      ? {
        id: target.id,
        type: "mock",
        destroyed: target.isDestroyed,
        focused: target.focused,
      }
      : null,
    scheduleFocusRestore: (callback: () => void) => {
      focusRestores.push(callback)
    },
    setHotkeysOpen: (open: boolean) => {
      hotkeysOpen = open
    },
    setTerminalPairingOpen: (open: boolean) => {
      terminalPairingOpen = open
    },
    setSessionBrowserOpen: (open: boolean) => {
      sessionBrowserOpen = open
    },
    setTerminalPairing: (pairing: TerminalPairingLinkView | null) => {
      terminalPairing = pairing
    },
    setTerminalPairingQrLines: (lines: string[]) => {
      terminalPairingQrLines = lines
    },
    getSessionCount: () => options.sessionCount ?? 2,
    getWaitingRoomSessionIndex: () => options.waitingRoomSessionIndex ?? 0,
    setSessionBrowserIndex: (index: number) => {
      sessionBrowserIndex = index
    },
    clampSessionBrowserIndex: (index: number, sessionCount: number) => Math.max(0, Math.min(index, sessionCount - 1)),
    renderOverlay: (mode: CliDialogOverlayMode) => {
      renderModes.push(mode)
    },
    createTerminalPairingLink: options.createTerminalPairingLink ?? (async () => pairingLink()),
    renderTerminalPairingQr: async () => ["qr"],
    flashFooter: (message: string, tone: "info" | "error") => {
      footerMessages.push({ message, tone })
    },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
  }

  return {
    deps,
    currentFocus,
    hotkeysOpen: () => hotkeysOpen,
    terminalPairingOpen: () => terminalPairingOpen,
    sessionBrowserOpen: () => sessionBrowserOpen,
    terminalPairing: () => terminalPairing,
    terminalPairingQrLines: () => terminalPairingQrLines,
    sessionBrowserIndex: () => sessionBrowserIndex,
    renderModes: () => renderModes,
    footerMessages: () => footerMessages,
    flushFocusRestores: () => {
      while (focusRestores.length > 0) {
        focusRestores.shift()?.()
      }
    },
  }
}

class MockFocusTarget implements CliDialogFocusTarget {
  isDestroyed = false
  focused = true
  blurCount = 0
  focusCount = 0

  constructor(readonly id: string) {}

  focus() {
    this.focused = true
    this.focusCount += 1
  }

  blur() {
    this.focused = false
    this.blurCount += 1
  }
}

function pairingLink(): TerminalPairingLinkView {
  return {
    terminal_id: "terminal-1",
    pairing_link: "https://example.test/pair",
    pairing_code: "123456",
    invite_id: "invite-1",
    relay_url: "wss://relay.example.test",
    target_daemon_id: "daemon-1",
    terminal_type: "cli",
    issued_at_ms: 1,
    expires_at_ms: 2,
  }
}
