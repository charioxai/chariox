import {
  BoxRenderable,
  TextareaRenderable,
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
import { buildWorkflowOutlineRenderable as buildWorkflowOutlinePaneRenderable } from "./workflow-outline/render.js"

type RenderContext = ReturnType<typeof useRenderer>

export function buildEmptyTranscriptRenderable(renderer: RenderContext) {
  return buildAsciiCanvasRenderable(renderer, "Type your first prompt below.", theme.textMuted)
}

export function buildWorkflowOutlineRenderable(
  renderer: RenderContext,
  options: {
    workflows: WorkflowDefinition[]
    agents: AgentInstance[]
    workflowRuns: WorkflowRun[]
    selectedWorkflowId: string | null
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
    inspector?: {
      title: string
      meta: string[]
      body?: string | null
      draft?: string
      placeholder?: string | null
      hint?: string | null
      onDraftChange?: ((draft: string) => void) | null
      onEditorRef?: ((editor: TextareaRenderable | null) => void) | null
    } | null
  },
) {
  const outline = buildWorkflowOutlinePaneRenderable(renderer, options)
  if (!outline) {
    return buildAsciiCanvasRenderable(renderer, "type /workflow to start creating a workflow", theme.warning)
  }
  if (!options.inspector) {
    return outline
  }
  const wrapper = new BoxRenderable(renderer, {
    flexDirection: "row",
    width: "100%",
    gap: 0,
  })
  const left = new BoxRenderable(renderer, {
    flexGrow: 2,
    minWidth: 40,
  })
  left.add(outline)
  wrapper.add(left)

  const inspectorConfig = options.inspector
  const inspector = new BoxRenderable(renderer, {
    flexGrow: 1,
    minWidth: 32,
    border: ["left"],
    borderColor: theme.borderSubtle,
    customBorderChars: SplitBorder.customBorderChars,
    paddingLeft: 1,
    paddingRight: 1,
    paddingTop: 1,
    paddingBottom: 1,
    flexDirection: "column",
    gap: 1,
  })
  inspector.add(
    new TextRenderable(renderer, {
      content: inspectorConfig.title,
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "word",
    }),
  )
  if (inspectorConfig.meta.length > 0) {
    inspector.add(
      new TextRenderable(renderer, {
        content: inspectorConfig.meta.join("\n"),
        fg: theme.textMuted,
        wrapMode: "word",
      }),
    )
  }
  if (inspectorConfig.onDraftChange) {
    const textarea = new TextareaRenderable(renderer, {
      flexGrow: 1,
      minHeight: 8,
      wrapMode: "word",
      initialValue: inspectorConfig.draft ?? "",
      backgroundColor: theme.backgroundPanel,
      textColor: theme.text,
      placeholder: inspectorConfig.placeholder ?? "Type instructions here",
      placeholderColor: theme.textMuted,
      onContentChange: () => {
        inspectorConfig.onDraftChange?.(textarea.plainText)
      },
    })
    textarea.onMouseUp = () => {
      textarea.focus()
    }
    inspectorConfig.onEditorRef?.(textarea)
    inspector.add(textarea)
  } else if (inspectorConfig.body) {
    inspector.add(
      new TextRenderable(renderer, {
        content: inspectorConfig.body,
        fg: theme.text,
        wrapMode: "word",
      }),
    )
  }
  if (inspectorConfig.hint) {
    inspector.add(
      new TextRenderable(renderer, {
        content: inspectorConfig.hint,
        fg: theme.textMuted,
        wrapMode: "word",
      }),
    )
  }
  wrapper.add(inspector)
  return wrapper
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
    const prefix = row.focused ? ">" : " "
    const indent = "  ".repeat(row.indent)
    const titleWidth = Math.max(24, row.titleWidth ?? 24)
    const value = row.columns ? ` ${row.columns.join("  ")}` : row.value ? ` ${row.value}` : ""
    const scrollbar = row.scrollbar ? `  ${row.scrollbar}` : ""
    const titleWidthSpace = Math.max(0, titleWidth - row.indent)
    menu.add(
      new TextRenderable(renderer, {
        content: `${prefix} ${indent}${row.title.padEnd(titleWidthSpace, " ")}${value}${scrollbar}`,
        fg: row.focused ? theme.primary : row.selectable ? theme.text : theme.textMuted,
        attributes: row.focused ? TextAttributes.BOLD : TextAttributes.NONE,
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
      content: `${SESSION_NEW_HELP_TEXT}\nUse ↑ ↓ to move through new-session options and saved sessions, ← → to adjust new-session choices, Enter to create or join.`,
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
