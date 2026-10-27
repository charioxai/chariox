import { BoxRenderable, TextAttributes, TextRenderable, type Renderable } from "@opentui/core"

import {
  buildDirectoryTreeRows,
  createDirectoryTreeState,
  isDirectoryTreePathLoaded,
  mergeDirectoryTreeEntries,
  moveDirectoryTreeSelection,
  toggleDirectoryTreeExpansion,
  type DirectoryTreeState,
} from "./tree-view.js"
import { readDirectoryTreeRequest } from "./ipc-requests.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ReadDirectoryTreeResult } from "./cli-types.js"
import { theme } from "./theme.js"

export const WORKTREE_VIEW_ENABLED = false

type TreeLoaderDeps = {
  client: LocalIpcClient
  sessionId: () => string
  attachmentId: () => string | null
  directoryTreeState: () => DirectoryTreeState | null
  setDirectoryTreeState: (state: DirectoryTreeState | null) => void
  setCenterMode: (mode: "transcript" | "tree") => void
  centerMode: () => "transcript" | "tree"
  rebuildTranscript: () => void
  flashFooter: (message: string, tone: "error" | "info") => void
  formatError: (error: unknown) => string
  getLastTranscriptScrollTop: () => number
  setLastTranscriptScrollTop: (value: number) => void
  transcriptScrollbox: () => {
    scrollTop: number
    scrollHeight: number
    height: number
    scrollLeft: number
    scrollTo: (position: { x: number; y: number }) => void
    requestRender: () => void
  } | undefined
}

export function createWorktreeViewController(deps: TreeLoaderDeps) {
  const loadDirectoryTree = async (treePath = "") => {
    const attachmentId = deps.attachmentId()
    if (!attachmentId) {
      return
    }
    const response = await deps.client.send<Record<string, unknown>>(
      readDirectoryTreeRequest(deps.sessionId(), attachmentId, treePath || null, 1),
    )
    const payload = expectVariant<{ result: ReadDirectoryTreeResult }>(response, "DirectoryTreeRead")
    const next = treePath && deps.directoryTreeState()
      ? mergeDirectoryTreeEntries(deps.directoryTreeState()!, treePath, payload.result.entries)
      : createDirectoryTreeState(payload.result.root_path, payload.result.entries)
    const previous = deps.directoryTreeState()
    if (previous && previous.rootPath === next.rootPath) {
      next.expandedPaths = previous.expandedPaths.filter((value) => value === "" || next.entries.some((entry) => entry.relative_path === value))
      next.selectedPath = next.entries.some((entry) => entry.relative_path === previous.selectedPath) || previous.selectedPath === ""
        ? previous.selectedPath
        : next.selectedPath
    }
    deps.setDirectoryTreeState(next)
  }

  const toggleCenterMode = async () => {
    if (!WORKTREE_VIEW_ENABLED) {
      return
    }
    if (deps.centerMode() === "tree") {
      deps.setCenterMode("transcript")
      deps.rebuildTranscript()
      const transcriptScrollbox = deps.transcriptScrollbox()
      if (transcriptScrollbox) {
        const maxScrollTop = Math.max(0, transcriptScrollbox.scrollHeight - transcriptScrollbox.height)
        const target = Math.max(0, Math.min(deps.getLastTranscriptScrollTop(), maxScrollTop))
        transcriptScrollbox.scrollTo({ x: transcriptScrollbox.scrollLeft, y: target })
        transcriptScrollbox.requestRender()
      }
      return
    }
    deps.setLastTranscriptScrollTop(deps.transcriptScrollbox()?.scrollTop ?? deps.getLastTranscriptScrollTop())
    try {
      await loadDirectoryTree()
    } catch (error) {
      deps.flashFooter(`failed to load tree: ${deps.formatError(error)}`, "error")
      return
    }
    deps.setCenterMode("tree")
    deps.rebuildTranscript()
  }

  const navigateDirectoryTree = (direction: "up" | "down") => {
    if (!WORKTREE_VIEW_ENABLED) {
      return
    }
    const state = deps.directoryTreeState()
    if (!state) {
      return
    }
    deps.setDirectoryTreeState(moveDirectoryTreeSelection(state, direction))
    deps.rebuildTranscript()
  }

  const activateDirectoryTreeSelection = () => {
    if (!WORKTREE_VIEW_ENABLED) {
      return
    }
    const state = deps.directoryTreeState()
    if (!state) {
      return
    }
    const row = buildDirectoryTreeRows(state).find((candidate) => candidate.id === state.selectedPath)
    if (!row || (row.kind !== "root" && row.kind !== "directory")) {
      return
    }
    const applyToggle = () => {
      const current = deps.directoryTreeState()
      deps.setDirectoryTreeState(current ? toggleDirectoryTreeExpansion(current) : current)
      deps.rebuildTranscript()
    }
    if (row.id === "" || isDirectoryTreePathLoaded(state, row.id)) {
      applyToggle()
      return
    }
    void loadDirectoryTree(row.id)
      .then(() => {
        applyToggle()
      })
      .catch((error) => {
        deps.flashFooter(`failed to load tree: ${deps.formatError(error)}`, "error")
      })
  }

  return {
    loadDirectoryTree,
    toggleCenterMode,
    navigateDirectoryTree,
    activateDirectoryTreeSelection,
  }
}

export function buildDirectoryTreeRenderable(renderer: {
  [key: string]: unknown
}, state: DirectoryTreeState) {
  const wrapper = new BoxRenderable(renderer as never, {
    marginBottom: 0,
    flexDirection: "column",
    gap: 0,
  })
  for (const row of buildDirectoryTreeRows(state)) {
    const isSelected = row.id === state.selectedPath
    const text = `${"  ".repeat(row.depth)}${row.kind === "root" || row.kind === "directory" ? (row.expanded ? "[-] " : "[+] ") : "    "}${row.label}`
    wrapper.add(
      new TextRenderable(renderer as never, {
        content: text,
        fg: row.kind === "root" ? theme.secondary : isSelected ? theme.primary : theme.text,
        ...(isSelected ? { bg: theme.backgroundElement } : {}),
        attributes: isSelected || row.kind === "root" ? TextAttributes.BOLD : TextAttributes.NONE,
        wrapMode: "none",
      }),
    )
  }
  return wrapper as unknown as Renderable
}

function expectVariant<TPayload>(response: Record<string, unknown>, expected: string) {
  const variant = response[expected]
  if (!variant || typeof variant !== "object") {
    throw new Error(`expected ${expected} response`)
  }
  return variant as TPayload
}
