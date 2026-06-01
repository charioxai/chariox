import { basename } from "node:path"

import type { ShellContext } from "./shell-core.js"
import {
  createSliceRequest,
  listSlicesRequest,
  startSliceRequest,
} from "./ipc-slice-requests.js"
import type { SliceRecord } from "./kernel-types.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellSlicePlacementDeps = {
  client: ShellKernelClient
}

export function shellSliceCreatesPlacement(sliceRef: string | undefined): boolean {
  return sliceRef === "new"
}

export async function resolveShellSliceRef(
  sliceSelection: string | undefined,
  context: Pick<ShellContext, "workspace" | "worktree">,
  worktree: string,
  deps: ShellSlicePlacementDeps,
  displayMode: "headless" | "headed" = "headless",
): Promise<string | undefined> {
  if (!sliceSelection || sliceSelection === "off") {
    return undefined
  }
  if (!shellSliceCreatesPlacement(sliceSelection)) {
    const slice = await resolveExistingShellSlice(sliceSelection, context, worktree, deps)
    if (slice.status !== "running") {
      await deps.client.send(startSliceRequest(slice.id))
    }
    return slice.id
  }
  const created = await deps.client.send(createSliceRequest({
    name: defaultShellSliceName(worktree),
    displayMode,
    workspaceId: context.workspace,
    worktreeId: worktree,
    workspaceMount: worktree,
  }))
  const slice = expectVariant<{ slice: SliceRecord }>(created, "SliceCreated").slice
  const started = await deps.client.send(startSliceRequest(slice.id))
  return expectVariant<{ slice: SliceRecord }>(started, "SliceStarted").slice.id
}

async function resolveExistingShellSlice(
  sliceRef: string,
  context: Pick<ShellContext, "workspace">,
  worktree: string,
  deps: ShellSlicePlacementDeps,
): Promise<SliceRecord> {
  const response = await deps.client.send(listSlicesRequest())
  const slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
  const slice = slices.find((candidate) => candidate.id === sliceRef || candidate.name === sliceRef)
  if (!slice) {
    throw new Error(`slice ${sliceRef} is not available for this kernel`)
  }
  if (slice.workspace_id !== context.workspace) {
    throw new Error(`slice ${sliceRef} is scoped to workspace ${slice.workspace_id ?? "unknown"}, not ${context.workspace}`)
  }
  const sliceWorktree = slice.worktree_id || slice.workspace_mount || ""
  if (sliceWorktree !== worktree) {
    throw new Error(`slice ${sliceRef} is scoped to worktree ${sliceWorktree || "unknown"}, not ${worktree}`)
  }
  return slice
}

function defaultShellSliceName(worktreePath: string): string {
  const leaf = basename(worktreePath) || "workspace"
  const suffix = Date.now().toString(36).slice(-5)
  return `${leaf}-slice-${suffix}`.replace(/[^a-zA-Z0-9_.-]/g, "-")
}

function expectVariant<T>(response: Record<string, unknown>, key: string): T {
  const value = response[key]
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`expected ${key} response`)
  }
  return value as T
}
