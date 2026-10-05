import { basename, resolve as resolvePath } from "node:path"

const CREATE_WORKTREE_OPTION_ID = "create-worktree"
const DEFAULT_CREATE_WORKTREE_LABEL = "Create worktree"

type WaitingRoomExistingWorktreeOption = {
  id: string
  kind: "existing"
  label: string
  path: string
  branch: string | null
  isCurrent: boolean
}

type WaitingRoomCreateWorktreeOption = {
  id: typeof CREATE_WORKTREE_OPTION_ID
  kind: "create"
  label: string
}

export type WaitingRoomWorktreeOption =
  | WaitingRoomExistingWorktreeOption
  | WaitingRoomCreateWorktreeOption

type WaitingRoomWorktreeInventory = {
  workspacePath: string
  currentWorktreePath: string
  options: WaitingRoomWorktreeOption[]
  disabledHint?: string | null
}

type PendingWaitingRoomWorktreeSelection =
  | { kind: "existing"; path: string }
  | { kind: "create" }

let activeInventory: WaitingRoomWorktreeInventory | null = null
let pendingSelection: PendingWaitingRoomWorktreeSelection | null = null

export function clearWaitingRoomWorktreeInventory() {
  activeInventory = null
  pendingSelection = null
}

export function clearStagedWaitingRoomWorktreeSelection() {
  pendingSelection = null
}

export function waitingRoomWorktreeOptions() {
  return activeInventory?.options ?? []
}

export function normalizeWaitingRoomWorktreeSelectionId(selectionId?: string | null) {
  const options = waitingRoomWorktreeOptions()
  if (selectionId && options.some((option) => option.id === selectionId)) {
    return selectionId
  }
  const current = options.find((option) => option.kind === "existing" && option.isCurrent)
  return current?.id ?? options[0]?.id ?? ""
}

export function cycleWaitingRoomWorktreeSelectionId(
  selectionId: string | null | undefined,
  delta: number,
) {
  const options = waitingRoomWorktreeOptions()
  if (options.length === 0) {
    return ""
  }
  const currentId = normalizeWaitingRoomWorktreeSelectionId(selectionId)
  const index = Math.max(0, options.findIndex((option) => option.id === currentId))
  return options[modulo(index + delta, options.length)]?.id ?? currentId
}

export function describeWaitingRoomWorktreeSelection(
  selectionId: string | null | undefined,
  fallbackPath?: string | null,
) {
  const disabledHint = waitingRoomWorktreeDisabledHint()
  if (disabledHint) return disabledHint
  const option = resolveWaitingRoomWorktreeOption(selectionId)
  if (option?.kind === "create") {
    return option.label
  }
  if (option?.kind === "existing") {
    return option.label
  }
  if (fallbackPath?.trim()) {
    return fallbackPath
  }
  return "Set worktree path"
}

export function selectedWaitingRoomWorktreePath(
  selectionId: string | null | undefined,
  fallbackPath?: string | null,
) {
  const option = resolveWaitingRoomWorktreeOption(selectionId)
  if (option?.kind === "existing") {
    return option.path
  }
  return fallbackPath?.trim() || activeInventory?.currentWorktreePath || ""
}

export function stageWaitingRoomWorktreeSelection(
  selectionId: string | null | undefined,
  fallbackPath?: string | null,
) {
  if (waitingRoomWorktreeDisabledHint()) {
    pendingSelection = null
    return { ok: true as const }
  }
  const option = resolveWaitingRoomWorktreeOption(selectionId)
  if (option?.kind === "create") {
    pendingSelection = { kind: "create" }
    return { ok: true as const }
  }
  if (option?.kind === "existing") {
    pendingSelection = { kind: "existing", path: option.path }
    return { ok: true as const }
  }
  if (fallbackPath?.trim()) {
    pendingSelection = { kind: "existing", path: fallbackPath.trim() }
    return { ok: true as const }
  }
  return {
    ok: false as const,
    message: "no worktree available for the new session",
  }
}

export async function resolvePendingWaitingRoomWorktreePath(
  workspacePath: string,
  fallbackWorktreePath: string,
  deps: {
    createWorktree?: (workspacePath: string) => Promise<string>
  } = {},
): Promise<string> {
  const selection = pendingSelection
  pendingSelection = null
  if (!selection || waitingRoomWorktreeDisabledHint()) {
    return fallbackWorktreePath
  }
  if (selection.kind === "existing") {
    return selection.path
  }
  if (!deps.createWorktree) throw new Error("kernel worktree creation is unavailable")
  return await deps.createWorktree(workspacePath)
}

export function __setWaitingRoomWorktreeInventoryForTest(options: {
  workspacePath: string
  currentWorktreePath: string
  options: WaitingRoomWorktreeOption[]
  disabledHint?: string | null
} | null) {
  activeInventory = options
  pendingSelection = null
}

type GitWorktreeEntry = { path: string; branch: string | null }

function formatWorktreeLabel(entry: GitWorktreeEntry, workspacePath: string) {
  if (samePath(entry.path, workspacePath)) {
    return "main"
  }
  if (entry.branch?.trim()) {
    return entry.branch
  }
  return basename(entry.path) || entry.path
}

function resolveWaitingRoomWorktreeOption(selectionId: string | null | undefined) {
  const normalizedSelectionId = normalizeWaitingRoomWorktreeSelectionId(selectionId)
  return waitingRoomWorktreeOptions().find((option) => option.id === normalizedSelectionId) ?? null
}

function samePath(left: string, right: string) {
  return resolvePath(left) === resolvePath(right)
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

export const NO_REPOSITORY_WORKTREE_HINT = "No git repository in this workspace — run git init to enable worktrees"
export const UNBORN_WORKTREE_HINT = "This repository has no commits — create a commit to enable worktrees"

export function waitingRoomWorktreeDisabledHint(): string | null {
  return activeInventory?.disabledHint ?? null
}

export function setWaitingRoomKernelWorktrees(
  workspacePath: string,
  currentWorktreePath: string,
  worktrees: readonly { path: string; branch?: string | null; label?: string | null; current: boolean }[],
  disabledHint: string | null,
) {
  activeInventory = {
    workspacePath,
    currentWorktreePath,
    disabledHint,
    options: disabledHint ? [] : [
      ...worktrees.map(worktree => ({
        id: `existing:${worktree.path}`,
        kind: "existing" as const,
        label: worktree.label || formatWorktreeLabel({ path: worktree.path, branch: worktree.branch ?? null }, workspacePath),
        path: worktree.path,
        branch: worktree.branch ?? null,
        isCurrent: worktree.current,
      })),
      { id: CREATE_WORKTREE_OPTION_ID, kind: "create" as const, label: DEFAULT_CREATE_WORKTREE_LABEL },
    ],
  }
}
