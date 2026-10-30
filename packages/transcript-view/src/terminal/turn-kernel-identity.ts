import {
  externalProviderObservedExactIdentityConflicts,
  externalProviderObservedExactIdentityMatches,
  externalProviderObservedIdentityKey,
  externalProviderObservedIdentityIsPresent,
  externalProviderObservedIdentityMatches,
  isExternalProviderObservedEntry,
} from "./external-provider-observation.js"
import {
  terminalPromptOriginFromRecord,
  terminalPromptOriginIsExternal,
} from "./prompt-origin.js"
import type {
  TerminalOutputEntry,
  TerminalTurnState,
} from "./runtime-state.js"

type KernelPromptOriginFields = {
  readonly promptId?: string | null
  readonly promptOrigin?: string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
}

export function assignExternalProviderTurnIdentity(
  turn: TerminalTurnState,
  options: {
    readonly externalProvider?: string | null
    readonly externalProviderSessionId?: string | null
    readonly externalProviderTurnId?: string | null
  },
): void {
  assignOptionalKernelString(turn, "kernelExternalProvider", options, "externalProvider")
  assignOptionalKernelString(turn, "kernelExternalProviderSessionId", options, "externalProviderSessionId")
  assignOptionalKernelString(turn, "kernelExternalProviderTurnId", options, "externalProviderTurnId")
}

export function assignKernelPromptId(
  turn: TerminalTurnState,
  options: { readonly promptId?: string | null },
): void {
  assignOptionalKernelString(turn, "kernelPromptId", options, "promptId")
}

export function assignKernelProviderRunId(
  turn: TerminalTurnState,
  options: { readonly providerRunId?: string | null },
): void {
  const providerRunId = options.providerRunId
  if (
    typeof providerRunId === "string"
    && turn.kernelProviderRunId?.startsWith("leased:")
    && terminalProviderRunIdsMatch(turn.kernelProviderRunId, providerRunId)
  ) {
    return
  }
  assignOptionalKernelString(turn, "kernelProviderRunId", options, "providerRunId")
}

export function assignKernelSourceAttachmentId(
  turn: TerminalTurnState,
  options: { readonly sourceAttachmentId?: string | null },
): void {
  if (!hasOwn(options, "sourceAttachmentId")) {
    return
  }
  const sourceAttachmentId = options.sourceAttachmentId
  if (sourceAttachmentId === undefined) {
    return
  }
  if (typeof sourceAttachmentId === "string") {
    turn.kernelSourceAttachmentId = sourceAttachmentId
    return
  }
  turn.kernelSourceAttachmentId = null
}

export function assignKernelPromptOrigin(
  turn: TerminalTurnState,
  options: KernelPromptOriginFields,
): void {
  if (!hasOwn(options, "promptOrigin")) {
    return
  }
  if (options.promptOrigin === undefined) {
    return
  }
  const promptOrigin = normalizedKernelPromptOrigin(options)
  if (promptOrigin) {
    turn.kernelPromptOrigin = promptOrigin
    return
  }
  delete turn.kernelPromptOrigin
}

export function normalizedKernelPromptOrigin(options: KernelPromptOriginFields): string | null {
  return terminalPromptOriginFromRecord({
    prompt_origin: options.promptOrigin ?? null,
  })
}

export function bindTerminalTurnKernelIdentityFromEntry(
  turn: TerminalTurnState,
  entry: Pick<TerminalOutputEntry,
    | "promptId"
    | "providerRunId"
    | "promptOrigin"
    | "sourceAttachmentId"
    | "source"
    | "externalProvider"
    | "externalProviderSessionId"
    | "externalProviderTurnId"
  >,
): void {
  turn.kernelObservedActive = true
  assignKernelPromptId(turn, entry)
  assignKernelProviderRunId(turn, entry)
  assignKernelSourceAttachmentId(turn, entry)
  assignKernelPromptOrigin(turn, entry)
  assignExternalProviderTurnIdentity(turn, {
    externalProvider: entry.externalProvider,
    externalProviderSessionId: entry.externalProviderSessionId,
    externalProviderTurnId: entry.externalProviderTurnId,
  })
}

export function terminalTurnHasNoKernelIdentity(
  turn: Pick<TerminalTurnState,
    | "kernelPromptId"
    | "kernelProviderRunId"
    | "kernelExternalProvider"
    | "kernelExternalProviderSessionId"
    | "kernelExternalProviderTurnId"
  >,
): boolean {
  return !turn.kernelPromptId
    && !turn.kernelProviderRunId
    && !turn.kernelExternalProvider
    && !turn.kernelExternalProviderSessionId
    && !turn.kernelExternalProviderTurnId
}

export function settlementHasKernelIdentity(
  options: {
    readonly promptId?: string | null
    readonly providerRunId?: string | null
    readonly externalProviderSessionId?: string | null
    readonly externalProviderTurnId?: string | null
  },
): boolean {
  return Boolean(
    options.promptId
      || options.providerRunId
      || externalProviderObservedIdentityIsPresent(options),
  )
}

export function terminalTurnMatchesHistory(
  turn: TerminalTurnState,
  turnId: string,
  options: {
    readonly promptId?: string | null
    readonly providerRunId?: string | null
    readonly sourceAttachmentId?: string | null
    readonly promptOrigin?: string | null
    readonly externalProvider?: string | null
    readonly externalProviderSessionId?: string | null
    readonly externalProviderTurnId?: string | null
  },
): boolean {
  const hasPromptId = Boolean(options.promptId)
  const hasProviderRunId = Boolean(options.providerRunId)
  const externalIdentity = {
    externalProvider: options.externalProvider,
    externalProviderSessionId: options.externalProviderSessionId,
    externalProviderTurnId: options.externalProviderTurnId,
  }
  const hasExternalIdentity = externalProviderObservedIdentityIsPresent(externalIdentity)
  const externalMatches = externalProviderObservedIdentityMatches({
    externalProvider: turn.kernelExternalProvider,
    externalProviderSessionId: turn.kernelExternalProviderSessionId,
    externalProviderTurnId: turn.kernelExternalProviderTurnId,
  }, externalIdentity)
  const exactExternalMatches = externalProviderObservedExactIdentityMatches({
    externalProvider: turn.kernelExternalProvider,
    externalProviderSessionId: turn.kernelExternalProviderSessionId,
    externalProviderTurnId: turn.kernelExternalProviderTurnId,
  }, externalIdentity)
  const exactExternalConflicts = externalProviderObservedExactIdentityConflicts({
    externalProvider: turn.kernelExternalProvider,
    externalProviderSessionId: turn.kernelExternalProviderSessionId,
    externalProviderTurnId: turn.kernelExternalProviderTurnId,
  }, externalIdentity)
  const promptMatches = !hasPromptId
    || !turn.kernelPromptId
    || turn.kernelPromptId === options.promptId
  const providerRunMatches = !hasProviderRunId
    || !turn.kernelProviderRunId
    || terminalProviderRunIdsMatch(turn.kernelProviderRunId, options.providerRunId ?? null)
  const sourceAttachmentMatches = !options.sourceAttachmentId
    || !turn.kernelSourceAttachmentId
    || turn.kernelSourceAttachmentId === options.sourceAttachmentId
  const exactSourceAttachmentMatches = Boolean(
    options.sourceAttachmentId
    && turn.kernelSourceAttachmentId === options.sourceAttachmentId,
  )
  if (exactExternalConflicts) {
    return false
  }
  if (turn.id === turnId) {
    return promptMatches && providerRunMatches && sourceAttachmentMatches && (!hasExternalIdentity || externalMatches)
  }
  if (hasPromptId && turn.kernelPromptId === options.promptId) {
    return providerRunMatches && sourceAttachmentMatches
  }
  if (hasProviderRunId && turn.kernelProviderRunId
    && terminalProviderRunIdsMatch(turn.kernelProviderRunId, options.providerRunId ?? null)) {
    return promptMatches && sourceAttachmentMatches
  }
  if (exactSourceAttachmentMatches) {
    // A source attachment is allocated for one kernel-owned prompt and remains
    // stable across dispatch and settlement. Remote dispatch failures use a
    // synthetic `remote-dispatch:*` run id while cancellation is projected as
    // `provider-run-cancelled`, so requiring those run ids to match would split
    // one failed prompt into two live turns.
    return promptMatches && (!hasExternalIdentity || externalMatches)
  }
  if (exactExternalMatches) {
    return promptMatches && providerRunMatches && sourceAttachmentMatches
  }
  return false
}

function terminalProviderRunIdsMatch(left: string, right: string | null): boolean {
  if (!right) {
    return true
  }
  if (left === right) {
    return true
  }
  const workerRunFromProjection = (value: string): string | null => {
    if (!value.startsWith("leased:")) {
      return null
    }
    const separator = value.lastIndexOf(":")
    return separator >= 0 && separator < value.length - 1
      ? value.slice(separator + 1)
      : null
  }
  return workerRunFromProjection(left) === right
    || workerRunFromProjection(right) === left
}

export function terminalTurnMatchesExternalProviderSession(
  turn: TerminalTurnState,
  entry: Pick<TerminalOutputEntry,
    | "externalProvider"
    | "externalProviderSessionId"
    | "externalProviderTurnId"
  >,
): boolean {
  const turnIdentity = externalProviderObservedIdentityKey({
    externalProvider: turn.kernelExternalProvider,
    externalProviderSessionId: turn.kernelExternalProviderSessionId,
    externalProviderTurnId: turn.kernelExternalProviderTurnId,
  })
  const entryIdentity = externalProviderObservedIdentityKey(entry)
  if (!turnIdentity || !entryIdentity) {
    return false
  }
  if (turnIdentity.provider && entryIdentity.provider && turnIdentity.provider !== entryIdentity.provider) {
    return false
  }
  return Boolean(entryIdentity.providerSessionId)
    && turnIdentity.providerSessionId === entryIdentity.providerSessionId
}

export function entryHasExternalTurnIdentity(
  entry: Pick<TerminalOutputEntry, "source" | "externalProvider" | "externalProviderSessionId" | "externalProviderTurnId">,
): boolean {
  return isExternalProviderObservedEntry(entry)
    && externalProviderObservedIdentityIsPresent(entry)
}

export function terminalTurnPromptOriginIsExternal(turn: Pick<TerminalTurnState, "kernelPromptOrigin">): boolean {
  return terminalPromptOriginIsExternal(turn.kernelPromptOrigin)
}

function assignOptionalKernelString<
  TargetKey extends
    | "kernelPromptId"
    | "kernelProviderRunId"
    | "kernelExternalProvider"
    | "kernelExternalProviderSessionId"
    | "kernelExternalProviderTurnId",
  SourceKey extends string,
>(
  turn: TerminalTurnState,
  targetKey: TargetKey,
  options: { readonly [key in SourceKey]?: string | null },
  sourceKey: SourceKey,
): void {
  if (!hasOwn(options, sourceKey)) {
    return
  }
  const value = options[sourceKey]
  if (value === undefined) {
    return
  }
  if (typeof value === "string") {
    turn[targetKey] = value
    return
  }
  delete turn[targetKey]
}

function hasOwn<T extends object, K extends PropertyKey>(
  value: T,
  key: K,
): value is T & Record<K, unknown> {
  return Object.prototype.hasOwnProperty.call(value, key)
}
