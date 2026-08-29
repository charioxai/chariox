import {
  BoxRenderable,
  RGBA,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type { HotkeySection } from "./hotkey-help.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"
import {
  formatPairingExpiry,
  formatTerminalTypeLabel,
  wrapPairingLink,
  type TerminalPairingLinkView,
} from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import { sessionBrowserCardLines } from "./session-browser-card.js"
import { theme } from "./theme.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import type { WaitingRoomRow } from "./waiting-room-types.js"
import { formatWaitingRoomMenuRow } from "./waiting-room-menu-row.js"

const HOTKEY_DIALOG_WIDTH = 72

export type CliDialogOverlayMode = "closed" | "managed-machine" | "session-browser" | "terminal-pairing" | "hotkeys"

type CliDialogOverlayOptions = {
  overlayBox: BoxRenderable | undefined
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  dimensions: { width: number; height: number }
  mode: CliDialogOverlayMode
  onDismiss: () => void
  sessions: SessionListEntry[]
  normalizeSessionBrowserIndex: () => number
  sessionBrowserProject?: WaitingRoomProjectSummary | null
  terminalPairing: TerminalPairingLinkView | null
  terminalPairingQrLines: string[]
  hotkeySections: HotkeySection[]
  managedMachineRows?: WaitingRoomRow[]
}

export function renderCliDialogOverlay(options: CliDialogOverlayOptions): void {
  const { overlayBox, renderer, dimensions } = options
  if (!overlayBox) {
    return
  }
  for (const child of [...overlayBox.getChildren()]) {
    overlayBox.remove(child.id)
    child.destroyRecursively()
  }
  if (options.mode === "closed") {
    overlayBox.requestRender()
    return
  }

  const scrim = new BoxRenderable(renderer, {
    position: "absolute",
    left: 0,
    top: 0,
    width: dimensions.width,
    height: dimensions.height,
    alignItems: "center",
    paddingTop: Math.max(1, Math.floor(dimensions.height / 5)),
    backgroundColor: RGBA.fromInts(0, 0, 0, 150),
  })
  scrim.onMouseUp = options.onDismiss

  if (options.mode === "managed-machine") {
    scrim.add(renderManagedMachinePanel(options))
  } else if (options.mode === "session-browser") {
    scrim.add(renderSessionBrowserPanel(options))
  } else if (options.mode === "terminal-pairing") {
    scrim.add(renderTerminalPairingPanel(options))
  } else {
    scrim.add(renderHotkeysPanel(options))
  }
  overlayBox.add(scrim)
  overlayBox.requestRender()
}

function renderManagedMachinePanel(options: CliDialogOverlayOptions): BoxRenderable {
  const { renderer, dimensions } = options
  const rows = options.managedMachineRows ?? []
  const width = Math.min(108, Math.max(76, Math.floor(dimensions.width * 0.82)))
  const panel = dialogPanel(renderer, width, dimensions.width)
  panel.add(dialogHeader(
    renderer,
    "New Chariox-managed machine",
    "↑/↓ fields • ←/→ values • Enter/Esc done",
  ))
  if (rows.length === 0) {
    panel.add(new TextRenderable(renderer, {
      content: "Managed-machine options are unavailable.",
      fg: theme.textMuted,
    }))
    return panel
  }
  const selectedIndex = Math.max(0, rows.findIndex((row) => row.focused))
  const maxRows = Math.max(1, Math.min(rows.length, dimensions.height - 9))
  const start = Math.min(
    Math.max(0, selectedIndex - maxRows + 1),
    Math.max(0, rows.length - maxRows),
  )
  for (const row of rows.slice(start, start + maxRows)) {
    panel.add(new TextRenderable(renderer, {
      content: formatWaitingRoomMenuRow(row, width - 4),
      fg: row.focused ? theme.primary : row.selectable ? theme.text : theme.textMuted,
      attributes: row.focused ? TextAttributes.BOLD : TextAttributes.NONE,
      wrapMode: "none",
    }))
  }
  if (rows.length > maxRows) {
    panel.add(new TextRenderable(renderer, {
      content: `${start + 1}-${Math.min(rows.length, start + maxRows)} of ${rows.length}`,
      fg: theme.textMuted,
    }))
  }
  return panel
}

function renderSessionBrowserPanel(options: CliDialogOverlayOptions): BoxRenderable {
  const { renderer, dimensions, sessions } = options
  const panel = dialogPanel(renderer, Math.min(112, Math.max(78, Math.floor(dimensions.width * 0.78))), dimensions.width)
  panel.add(dialogHeader(
    renderer,
    options.sessionBrowserProject ? `${options.sessionBrowserProject.name} Sessions` : "All Sessions",
    "Enter opens • A/D confirm • Esc returns",
  ))
  if (sessions.length === 0) {
    panel.add(new TextRenderable(renderer, {
      content: "No sessions available.",
      fg: theme.textMuted,
    }))
    return panel
  }

  const index = options.normalizeSessionBrowserIndex()
  const availablePanelHeight = dimensions.height - Math.max(1, Math.floor(dimensions.height / 5))
  const maxRows = Math.max(1, Math.min(5, Math.floor((availablePanelHeight - 5) / 6)))
  const start = Math.min(Math.max(0, index - maxRows + 1), Math.max(0, sessions.length - maxRows))
  for (const [offset, session] of sessions.slice(start, start + maxRows).entries()) {
    const rowIndex = start + offset
    const selected = rowIndex === index
    const lines = sessionBrowserCardLines(session, selected)
    const card = new BoxRenderable(renderer, {
      flexDirection: "column",
    })
    card.add(new TextRenderable(renderer, {
      content: lines.title,
      fg: selected ? theme.primary : theme.text,
      ...(selected ? { attributes: TextAttributes.BOLD } : {}),
      wrapMode: "none",
    }))
    card.add(new TextRenderable(renderer, {
      content: lines.timestamps,
      fg: theme.textMuted,
      wrapMode: "none",
    }))
    card.add(new TextRenderable(renderer, {
      content: lines.agents,
      fg: selected ? theme.text : theme.textMuted,
      wrapMode: "none",
    }))
    card.add(new TextRenderable(renderer, {
      content: lines.workflows,
      fg: theme.textMuted,
      wrapMode: "none",
    }))
    card.add(new TextRenderable(renderer, {
      content: lines.collaborations,
      fg: theme.textMuted,
      wrapMode: "none",
    }))
    panel.add(card)
  }
  if (sessions.length > maxRows) {
    panel.add(new TextRenderable(renderer, {
      content: `${start + 1}-${Math.min(sessions.length, start + maxRows)} of ${sessions.length}`,
      fg: theme.textMuted,
    }))
  }
  return panel
}

function renderTerminalPairingPanel(options: CliDialogOverlayOptions): BoxRenderable {
  const { renderer, dimensions, terminalPairing } = options
  const panel = dialogPanel(renderer, Math.min(96, Math.max(72, Math.floor(dimensions.width * 0.72))), dimensions.width)
  panel.add(dialogHeader(renderer, "Add New Terminal", "Esc closes"))
  if (!terminalPairing) {
    panel.add(new TextRenderable(renderer, {
      content: "Creating pairing link...",
      fg: theme.textMuted,
    }))
    return panel
  }

  panel.add(new TextRenderable(renderer, {
    content: `Type: ${formatTerminalTypeLabel(terminalPairing.terminal_type)}   Code: ${terminalPairing.pairing_code}`,
    fg: theme.primary,
    attributes: TextAttributes.BOLD,
  }))
  panel.add(new TextRenderable(renderer, {
    content: `Expires: ${formatPairingExpiry(terminalPairing.expires_at_ms)}`,
    fg: theme.textMuted,
  }))
  if (options.terminalPairingQrLines.length > 0) {
    panel.add(new TextRenderable(renderer, {
      content: options.terminalPairingQrLines.join("\n"),
      fg: theme.text,
    }))
  }
  panel.add(new TextRenderable(renderer, {
    content: "Pairing link",
    fg: theme.primary,
    attributes: TextAttributes.BOLD,
  }))
  for (const line of wrapPairingLink(terminalPairing.pairing_link, Math.max(36, Math.min(88, dimensions.width - 10)))) {
    panel.add(new TextRenderable(renderer, {
      content: line,
      fg: theme.text,
    }))
  }
  return panel
}

function renderHotkeysPanel(options: CliDialogOverlayOptions): BoxRenderable {
  const { renderer, dimensions } = options
  const panel = dialogPanel(renderer, HOTKEY_DIALOG_WIDTH, dimensions.width)
  panel.add(dialogHeader(renderer, "Hotkeys", "Esc closes"))
  panel.add(new TextRenderable(renderer, {
    content: `${HOTKEY_TOGGLE_LABEL} toggles this list.`,
    fg: theme.textMuted,
  }))
  for (const section of options.hotkeySections) {
    const sectionBox = new BoxRenderable(renderer, {
      flexDirection: "column",
      gap: 1,
    })
    sectionBox.add(new TextRenderable(renderer, {
      content: section.title,
      attributes: TextAttributes.BOLD,
      fg: theme.primary,
    }))
    for (const item of section.items) {
      const row = new BoxRenderable(renderer, {
        flexDirection: "row",
        gap: 2,
      })
      const keys = new BoxRenderable(renderer, {
        width: 22,
        flexShrink: 0,
      })
      keys.add(new TextRenderable(renderer, {
        content: item.keys,
        attributes: TextAttributes.BOLD,
        fg: theme.text,
      }))
      row.add(keys)
      row.add(new TextRenderable(renderer, {
        content: item.description,
        fg: theme.textMuted,
      }))
      sectionBox.add(row)
    }
    panel.add(sectionBox)
  }
  return panel
}

function dialogPanel(
  renderer: ConstructorParameters<typeof BoxRenderable>[0],
  width: number,
  viewportWidth: number,
): BoxRenderable {
  const panel = new BoxRenderable(renderer, {
    width,
    maxWidth: viewportWidth - 4,
    backgroundColor: theme.backgroundPanel,
    paddingTop: 1,
    paddingBottom: 1,
    paddingLeft: 2,
    paddingRight: 2,
    flexDirection: "column",
    gap: 1,
  })
  panel.onMouseUp = (event) => {
    event.stopPropagation()
  }
  return panel
}

function dialogHeader(
  renderer: ConstructorParameters<typeof BoxRenderable>[0],
  title: string,
  hint: string,
): BoxRenderable {
  const header = new BoxRenderable(renderer, {
    flexDirection: "row",
    justifyContent: "space-between",
  })
  header.add(new TextRenderable(renderer, {
    content: title,
    attributes: TextAttributes.BOLD,
    fg: theme.text,
  }))
  header.add(new TextRenderable(renderer, {
    content: hint,
    fg: theme.textMuted,
  }))
  return header
}
