import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
  type RGBA,
} from "@opentui/core"
import type { useRenderer } from "@opentui/solid"

import type {
  AgentInstance,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowRun,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { SESSION_NEW_HELP_TEXT } from "./sessions.js"
import { SplitBorder, theme } from "./theme.js"
import type { WaitingRoomState } from "./waiting-room.js"
import { arrobaArtFrame, waitingRoomRows } from "./waiting-room.js"
import { buildWorkflowCanvasRenderable as buildWorkflowCanvasPaneRenderable } from "./workflow-graph/canvas.js"

type RenderContext = ReturnType<typeof useRenderer>

export function buildEmptyTranscriptRenderable(renderer: RenderContext) {
  return buildAsciiCanvasRenderable(renderer, "Type your first prompt below.", theme.textMuted)
}

export function buildWorkflowCanvasRenderable(
  renderer: RenderContext,
  options: {
    workflows: WorkflowDefinition[]
    agents: AgentInstance[]
    workflowRuns: WorkflowRun[]
    selectedWorkflowId: string | null
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
  },
) {
  return buildWorkflowCanvasPaneRenderable(renderer, options)
    ?? buildAsciiCanvasRenderable(renderer, "type /workflow to start creating a workflow", theme.warning)
}

export function buildNoSessionRenderable(
  renderer: RenderContext,
  state: WaitingRoomState,
  sessions: RuntimeSession[],
  catalog: ProviderCatalog,
) {
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    flexGrow: 1,
    flexDirection: "column",
    justifyContent: "center",
    alignItems: "center",
    gap: 1,
  })
  const rows = waitingRoomRows(state, sessions, catalog)
  wrapper.add(
    new TextRenderable(renderer, {
      content: arrobaArtFrame(state.introStep),
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "none",
    }),
  )
  wrapper.add(
    new TextRenderable(renderer, {
      content: "No session attached. Dial in and choose your next run.",
      fg: theme.warning,
      wrapMode: "word",
    }),
  )
  const menu = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 0,
    border: ["left"],
    borderColor: theme.secondary,
    customBorderChars: SplitBorder.customBorderChars,
    paddingLeft: 1,
  })
  for (const row of rows) {
    menu.add(
      new TextRenderable(renderer, {
        content: `${state.focus === row.id ? ">" : " "} ${row.title.padEnd(22, " ")} ${row.value}`,
        fg: state.focus === row.id ? theme.primary : theme.text,
        attributes: state.focus === row.id ? TextAttributes.BOLD : TextAttributes.NONE,
        wrapMode: "none",
      }),
    )
  }
  wrapper.add(menu)
  wrapper.add(
    new TextRenderable(renderer, {
      content: renderWaitingRoomKeys(state),
      fg: theme.textMuted,
      wrapMode: "none",
    }),
  )
  wrapper.add(
    new TextRenderable(renderer, {
      content: `${SESSION_NEW_HELP_TEXT}\nUse ↑ ↓ to move, ← → to cycle, Enter to confirm.`,
      fg: theme.textMuted,
      wrapMode: "word",
    }),
  )
  return wrapper
}

function buildAsciiCanvasRenderable(
  renderer: RenderContext,
  promptText: string,
  promptColor: RGBA,
) {
  const art = arrobaArtFrame(12)
  const prompt = centerTextToWidth(promptText, maxLineWidth(art))
  const wrapper = new BoxRenderable(renderer, {
    marginBottom: 0,
    flexDirection: "column",
    gap: 1,
    paddingLeft: 2,
    paddingRight: 2,
    paddingTop: 2,
  })
  const artContainer = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 1,
    alignItems: "center",
    width: "100%",
  })
  artContainer.add(
    new TextRenderable(renderer, {
      content: art,
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "none",
    }),
  )
  artContainer.add(
    new TextRenderable(renderer, {
      content: prompt,
      fg: promptColor,
      wrapMode: "none",
    }),
  )
  wrapper.add(artContainer)
  return wrapper
}

function maxLineWidth(content: string) {
  return content.split("\n").reduce((width, line) => Math.max(width, line.length), 0)
}

function centerTextToWidth(content: string, width: number) {
  if (content.length >= width) {
    return content
  }
  const padding = width - content.length
  const leftPadding = Math.floor(padding / 2)
  const rightPadding = padding - leftPadding
  return `${" ".repeat(leftPadding)}${content}${" ".repeat(rightPadding)}`
}

function renderWaitingRoomKeys(state: WaitingRoomState) {
  const key = (label: string, pressed: boolean) => (pressed ? `[${label}]` : `<${label}>`)
  return [
    `        ${key("^", state.keyState.up)}`,
    `${key("<", state.keyState.left)} ${key("v", state.keyState.down)} ${key(">", state.keyState.right)}`,
  ].join("\n")
}
