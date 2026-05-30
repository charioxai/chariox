import { basename } from "node:path"

import type { ShellContext } from "./shell-core.js"
import {
  createSliceRequest,
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
): Promise<string | undefined> {
  if (!sliceSelection || sliceSelection === "off") {
    return undefined
  }
  if (!shellSliceCreatesPlacement(sliceSelection)) {
    await deps.client.send(startSliceRequest(sliceSelection))
    return sliceSelection
  }
  const created = await deps.client.send(createSliceRequest({
    name: defaultShellSliceName(worktree),
    displayMode: "headless",
    workspaceId: context.workspace,
    worktreeId: worktree,
    workspaceMount: worktree,
  }))
  const slice = expectVariant<{ slice: SliceRecord }>(created, "SliceCreated").slice
  const started = await deps.client.send(startSliceRequest(slice.id))
  return expectVariant<{ slice: SliceRecord }>(started, "SliceStarted").slice.id
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
