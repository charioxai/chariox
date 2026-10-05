import { execFile } from "node:child_process"
import { basename, resolve as resolvePath } from "node:path"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)
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

export async function primeWaitingRoomWorktreeInventory(options: {
  cwd: string
  workspacePath: string
  currentWorktreePath: string
}): Promise<void> {
  activeInventory = await discoverWaitingRoomWorktreeInventory(options)
  pendingSelection = null
}

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
  return current?.id ?? options[0]?.id ?? CREATE_WORKTREE_OPTION_ID
}

export function cycleWaitingRoomWorktreeSelectionId(
  selectionId: string | null | undefined,
  delta: number,
) {
  const options = waitingRoomWorktreeOptions()
  if (options.length === 0) {
    return selectionId ?? CREATE_WORKTREE_OPTION_ID
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

async function discoverWaitingRoomWorktreeInventory(options: {
  cwd: string
  workspacePath: string
  currentWorktreePath: string
}): Promise<WaitingRoomWorktreeInventory> {
  const gitCwd = options.currentWorktreePath || options.workspacePath || options.cwd
  const repository = await execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd: gitCwd }).then(() => true).catch(() => false)
  const hasCommit = repository && await execFileAsync("git", ["rev-parse", "--verify", "HEAD^{commit}"], { cwd: gitCwd }).then(() => true).catch(() => false)
  const disabledHint = !repository ? NO_REPOSITORY_WORKTREE_HINT : !hasCommit ? UNBORN_WORKTREE_HINT : null
  if (disabledHint) return { workspacePath: options.workspacePath, currentWorktreePath: options.currentWorktreePath, options: [], disabledHint }
  const discovered = await listGitWorktrees(gitCwd).catch(() => [])
  const existingOptions = discovered.length > 0
    ? discovered.map((entry) => ({
      id: `existing:${entry.path}`,
      kind: "existing" as const,
      label: formatWorktreeLabel(entry, options.workspacePath),
      path: entry.path,
      branch: entry.branch,
      isCurrent: samePath(entry.path, options.currentWorktreePath),
    }))
    : [{
      id: `existing:${options.currentWorktreePath}`,
      kind: "existing" as const,
      label: formatFallbackWorktreeLabel(options.currentWorktreePath, options.workspacePath),
      path: options.currentWorktreePath,
      branch: null,
      isCurrent: true,
    }]

  const sortedExistingOptions = [
    ...existingOptions.filter((option) => option.label === "main"),
    ...existingOptions.filter((option) => option.label !== "main"),
  ]

  return {
    workspacePath: options.workspacePath,
    currentWorktreePath: options.currentWorktreePath,
    options: [
      ...sortedExistingOptions,
      {
        id: CREATE_WORKTREE_OPTION_ID,
        kind: "create",
        label: DEFAULT_CREATE_WORKTREE_LABEL,
      },
    ],
  }
}

type GitWorktreeEntry = {
  path: string
  branch: string | null
}

async function listGitWorktrees(cwd: string): Promise<GitWorktreeEntry[]> {
  const { stdout } = await execFileAsync("git", ["worktree", "list", "--porcelain"], { cwd })
  return parseGitWorktreeList(stdout)
}

function parseGitWorktreeList(stdout: string): GitWorktreeEntry[] {
  const entries: GitWorktreeEntry[] = []
  let current: GitWorktreeEntry | null = null

  for (const rawLine of stdout.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line) {
      if (current?.path) {
        entries.push(current)
      }
      current = null
      continue
    }
    if (line.startsWith("worktree ")) {
      if (current?.path) {
        entries.push(current)
      }
      current = {
        path: line.slice("worktree ".length).trim(),
        branch: null,
      }
      continue
    }
    if (line.startsWith("branch ") && current) {
      current.branch = normalizeGitBranch(line.slice("branch ".length).trim())
    }
  }

  if (current?.path) {
    entries.push(current)
  }
  return entries
}

function normalizeGitBranch(ref: string) {
  return ref.replace(/^refs\/heads\//, "") || null
}

function formatWorktreeLabel(entry: GitWorktreeEntry, workspacePath: string) {
  if (samePath(entry.path, workspacePath)) {
    return "main"
  }
  if (entry.branch?.trim()) {
    return entry.branch
  }
  return basename(entry.path) || entry.path
}

function formatFallbackWorktreeLabel(worktreePath: string, workspacePath: string) {
  if (samePath(worktreePath, workspacePath)) {
    return "main"
  }
  return basename(worktreePath) || worktreePath
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
