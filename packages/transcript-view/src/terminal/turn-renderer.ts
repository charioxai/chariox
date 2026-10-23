import type {
  TerminalOutputEntry,
  TerminalTurnState,
} from "./runtime-state.js"
function formatLocalInstant(timestampMs: number, options: Intl.DateTimeFormatOptions): string {
  return Number.isFinite(timestampMs) ? new Intl.DateTimeFormat(undefined, options).format(timestampMs) : ""
}
import {
  renderFreeformChevronIcon,
  renderFreeformBlobHeader,
} from "./freeform-transcript-renderer.js"
import {
  terminalTurnActivitySummary,
} from "./activity-group-projection.js"
import {
  mergeTerminalToolCallAndOutputSnapshots,
  terminalToolCallId,
  terminalToolSnapshotIdentities,
  terminalToolSnapshotStatus,
} from "./tool-call-identity.js"
import {
  externalProviderObservedExactIdentityConflicts,
  externalProviderObservedEntryTurnMetadata,
  externalProviderObservedTurnMarker,
  type ExternalProviderObservedIdentityFields,
  type ExternalProviderObservedTurnMarker,
} from "./external-provider-observation.js"
import {
  terminalTurnExpansionEntryKey,
} from "./runtime-state.js"
import {
  terminalOutputEntryShouldRenderInTurn,
} from "./terminal-output-render-policy.js"
import {
  hasTerminalTurnId,
  terminalTurnCompletionSortValue,
  terminalTurnExpandedForDisplay,
  terminalTurnIsCompleted,
} from "./turn-state.js"
import {
  terminalTurnPromptOriginIsExternal,
} from "./turn-kernel-identity.js"
import {
  terminalPromptOriginIsExternal,
  terminalPromptTextIsPrivateContinuation,
} from "./prompt-origin.js"

export type TerminalTurnRenderOptions = {
  readonly turns: ReadonlyMap<string, TerminalTurnState>
  readonly expandedEntryKeys?: ReadonlySet<string>
  readonly completedTurnForAgent?: (agentId: string) => TerminalTurnCompletedAction | null
  /** False on pages that cannot undo or fork a turn: the buttons are omitted. */
  readonly turnActions?: boolean
  readonly renderEntry: (entry: TerminalOutputEntry) => string
  readonly renderTranscriptHtml: (text: string) => string
  readonly escapeHtml: (value: string) => string
  readonly formatDateTime?: (timestampMs: number) => string
}

export type TerminalTurnCompletedAction = {
  readonly turnId: string
  readonly promptId: string
  readonly providerRunId?: string | null
  readonly agentId: string
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
  readonly completedAtMs?: number
  readonly undoAvailable: boolean
  readonly undoUnavailableReason?: string | null
}

type TerminalRenderGroup =
  | { readonly kind: "entry"; readonly entry: TerminalOutputEntry }
  | TerminalTurnRenderGroup

type TerminalTurnRenderGroup = {
  readonly kind: "turn"
  turn: TerminalTurnState
  readonly entries: TerminalOutputEntry[]
  readonly segmentKey: string
}


export function renderTerminalEntriesByTurn(
  entries: TerminalOutputEntry[],
  options: TerminalTurnRenderOptions,
): string {
  entries = deferOrphanedCrossRunSummaries(entries, options.turns)
  const groups: TerminalRenderGroup[] = []
  const turnSegmentCounts = new Map<string, number>()
  let currentTurn: TerminalTurnState | null = null
  let currentSegmentTurnGroups = new Map<string, TerminalTurnRenderGroup>()
  for (const entry of entries) {
    if (!terminalOutputEntryShouldRenderInTurn(entry)) {
      continue
    }
    // MP-08/MP-09/MP-10/MP-11 G17: public task notices outlive provider turns.
    if (entry.turnTracking === "none" || entry.role === "status") {
      currentSegmentTurnGroups = new Map()
      groups.push({ kind: "entry", entry })
      continue
    }
    // A steer is part of the turn it steers: it never opens a new section,
    // so the turn's later output stays in the same section below it.
    const steersCurrentTurn: boolean = currentTurn !== null && terminalOutputEntryIsSteeringPrompt(entry)
    const explicitTurn: TerminalTurnState | null = !steersCurrentTurn && hasTerminalTurnId(entry.turnId)
      ? options.turns.get(entry.turnId) ?? null
      : null
    const turn: TerminalTurnState | null = steersCurrentTurn ? currentTurn : explicitTurn ?? (entry.role === "user" ? null : currentTurn)
    if (explicitTurn) {
      currentTurn = explicitTurn
    } else if (entry.role === "user" && !steersCurrentTurn) {
      currentTurn = null
      currentSegmentTurnGroups = new Map()
    }
    if (!turn) {
      groups.push({ kind: "entry", entry })
      continue
    }
    let group = currentSegmentTurnGroups.get(turn.id) ?? null
    if (!group) {
      const segmentIndex: number = turnSegmentCounts.get(turn.id) ?? 0
      turnSegmentCounts.set(turn.id, segmentIndex + 1)
      const segmentKey: string = segmentIndex === 0 ? turn.id : `${turn.id}:segment-${segmentIndex + 1}`
      group = { kind: "turn", turn, entries: [], segmentKey }
      currentSegmentTurnGroups.set(turn.id, group)
      groups.push(group)
    }
    group.entries.push(entry)
  }
  return foldDuplicateProviderRunGroups(foldPrivateContinuationGroups(groups)).map((group) => group.kind === "entry"
    ? options.renderEntry(group.entry)
    : renderTerminalTurn(group.turn, group.entries, options, group.segmentKey)).join("")
}

function foldDuplicateProviderRunGroups(groups: TerminalRenderGroup[]): TerminalRenderGroup[] {
  const folded = new Set<TerminalTurnRenderGroup>()
  const preceding: TerminalTurnRenderGroup[] = []
  for (const group of groups) {
    if (group.kind !== "turn") {
      continue
    }
    const runIds = [...new Set(group.entries.map((entry) => entry.providerRunId).filter((id): id is string => Boolean(id)))]
    const hasPrompt = group.entries.some((entry) => entry.role === "user")
    if (!hasPrompt && runIds.length === 1 && terminalTurnIsCompleted(group.turn)) {
      const target = preceding.slice().reverse().find((candidate) => (
        candidate.turn.id !== group.turn.id
        && candidate.turn.agentId === group.turn.agentId
        && candidate.turn.startedAtMs === group.turn.startedAtMs
        && terminalTurnIsCompleted(candidate.turn)
        && candidate.entries.some((entry) => entry.role === "user")
        && candidate.entries.some((entry) => entry.providerRunId === runIds[0])
        && candidate.entries.every((entry) => !entry.providerRunId || entry.providerRunId === runIds[0])
      ))
      if (target) {
        target.entries.push(...group.entries)
        target.turn = {
          ...target.turn,
          lastEntryAtMs: Math.max(target.turn.lastEntryAtMs, group.turn.lastEntryAtMs),
          completedAtMs: Math.max(target.turn.completedAtMs ?? 0, group.turn.completedAtMs ?? 0),
        }
        folded.add(group)
        continue
      }
    }
    preceding.push(group)
  }
  return groups.filter((group) => group.kind !== "turn" || !folded.has(group))
}

function deferOrphanedCrossRunSummaries(
  entries: readonly TerminalOutputEntry[],
  turns: ReadonlyMap<string, TerminalTurnState>,
): TerminalOutputEntry[] {
  // A reconnect can restore output before its lifecycle turn record. When a
  // later provider run continues that orphaned turn, keep its activity above
  // the prior run's terminal summary until canonical turn state arrives.
  const assistantRuns = new Map<string, {
    authoritativeSummaryIndexes: number[]
    finalAssistantIndex: number
  }>()
  let promptSegment = 0
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index]
    if (entry.role === "user" && entry.promptRole !== "steering") {
      promptSegment += 1
    }
    if (
      entry.role !== "assistant"
      || !hasTerminalTurnId(entry.turnId)
      || turns.has(entry.turnId)
      || !entry.providerRunId
    ) {
      continue
    }
    const runKey = JSON.stringify([promptSegment, entry.turnId, entry.providerRunId])
    const run = assistantRuns.get(runKey) ?? {
      authoritativeSummaryIndexes: [],
      finalAssistantIndex: index,
    }
    run.finalAssistantIndex = index
    if (entry.historyTurnSummary) {
      run.authoritativeSummaryIndexes.push(index)
    }
    assistantRuns.set(runKey, run)
  }

  const summaryIndexes = [...assistantRuns.values()]
    .flatMap((run) => run.authoritativeSummaryIndexes.length > 0
      ? run.authoritativeSummaryIndexes
      : [run.finalAssistantIndex])
    .sort((left, right) => left - right)
  const movedSummaryIndexes = new Set<number>()
  const summaryIndexesByActivityIndex = new Map<number, number[]>()
  for (const summaryIndex of summaryIndexes) {
    const summary = entries[summaryIndex]
    const turnId = summary.turnId
    const providerRunId = summary.providerRunId
    let lastContinuationActivityIndex = -1
    for (let index = summaryIndex + 1; index < entries.length; index += 1) {
      const candidate = entries[index]
      if (candidate.role === "user" && candidate.promptRole !== "steering") {
        break
      }
      if (candidate.turnId !== turnId) {
        continue
      }
      if (
        (candidate.role === "tool" || candidate.role === "reasoning")
        && candidate.providerRunId
        && candidate.providerRunId !== providerRunId
      ) {
        lastContinuationActivityIndex = index
      }
    }
    if (lastContinuationActivityIndex < 0) {
      continue
    }
    movedSummaryIndexes.add(summaryIndex)
    const summaries = summaryIndexesByActivityIndex.get(lastContinuationActivityIndex) ?? []
    summaries.push(summaryIndex)
    summaryIndexesByActivityIndex.set(lastContinuationActivityIndex, summaries)
  }

  const ordered: TerminalOutputEntry[] = []
  for (let index = 0; index < entries.length; index += 1) {
    if (!movedSummaryIndexes.has(index)) {
      ordered.push(entries[index])
    }
    const deferredSummaryIndexes = summaryIndexesByActivityIndex.get(index) ?? []
    for (const summaryIndex of deferredSummaryIndexes) {
      ordered.push(entries[summaryIndex])
    }
  }
  return ordered
}

function foldPrivateContinuationGroups(
  groups: readonly TerminalRenderGroup[],
): TerminalRenderGroup[] {
  const turnGroups = groups.filter((group): group is TerminalTurnRenderGroup => group.kind === "turn")
  const foldedGroups = new Set<TerminalTurnRenderGroup>()
  for (const continuationGroup of turnGroups) {
    const privatePrompt = continuationGroup.entries.find((entry) => (
      entry.role === "user"
      && entry.turnTracking !== "none"
      && terminalOutputEntryIsPrivateContinuationPrompt(entry)
    ))
    if (!privatePrompt) {
      continue
    }
    const continuationAttachmentIds = new Set(
      continuationGroup.entries
        .filter((entry) => entry !== privatePrompt)
        .map((entry) => entry.sourceAttachmentId)
        .filter((attachmentId): attachmentId is string => Boolean(attachmentId)),
    )
    if (continuationAttachmentIds.size === 0) {
      continue
    }
    const targetCandidates = turnGroups
      .filter((candidate) => (
        candidate !== continuationGroup
        && candidate.turn.agentId === continuationGroup.turn.agentId
        && candidate.turn.startedAtMs <= continuationGroup.turn.startedAtMs
        && candidate.entries.some((entry) => (
          entry.role === "user"
          && !terminalOutputEntryIsPrivateContinuationPrompt(entry)
        ))
      ))
    const targetGroup = (
      targetCandidates
        .filter((candidate) => candidate.turn.kernelSettlementStatus === "cancelled")
        .sort((left, right) => right.turn.startedAtMs - left.turn.startedAtMs)
        .at(0)
      ?? targetCandidates
        .filter((candidate) => candidate.entries.some((entry) => (
          Boolean(
            entry.sourceAttachmentId
            && continuationAttachmentIds.has(entry.sourceAttachmentId),
          )
        )))
        .sort((left, right) => right.turn.startedAtMs - left.turn.startedAtMs)
        .at(0)
      ?? targetCandidates
        .sort((left, right) => right.turn.startedAtMs - left.turn.startedAtMs)
        .at(0)
    )
    if (!targetGroup) {
      continue
    }
    targetGroup.entries.push(...continuationGroup.entries.filter((entry) => (
      entry !== privatePrompt && entry.role !== "reasoning"
    )))
    targetGroup.turn = {
      ...targetGroup.turn,
      lastEntryAtMs: Math.max(
        targetGroup.turn.lastEntryAtMs,
        continuationGroup.turn.lastEntryAtMs,
      ),
      completedAtMs: continuationGroup.turn.completedAtMs,
      kernelObservedActive: continuationGroup.turn.kernelObservedActive,
      kernelSettled: continuationGroup.turn.kernelSettled,
      kernelSettlementStatus: continuationGroup.turn.kernelSettlementStatus,
    }
    foldedGroups.add(continuationGroup)
  }
  return groups.filter((group) => group.kind !== "turn" || !foldedGroups.has(group))
}

export function renderTerminalTurn(
  turn: TerminalTurnState,
  entries: TerminalOutputEntry[],
  options: TerminalTurnRenderOptions,
  segmentKey = turn.id,
): string {
  const promptEntry = entries.find((entry) => entry.role === "user" && entry.turnTracking !== "none" && !terminalOutputEntryIsSteeringPrompt(entry))
  const privateContinuation = promptEntry ? terminalOutputEntryIsPrivateContinuationPrompt(promptEntry) : false
  const prompt = promptEntry && !privateContinuation && (promptEntry.text.trim() || promptEntry.attachments?.length)
    ? promptEntry
    : null
  const internalContinuation = Boolean(promptEntry && (privateContinuation || !prompt))
  const bodyEntries = entries.filter((entry) => entry !== promptEntry)
  const renderableBodyEntries = bodyEntries.filter(terminalOutputEntryShouldRenderInTurn)
  const taskNotices = renderableBodyEntries.filter((entry) => entry.role === "status")
  const completed = terminalTurnIsCompleted(turn)
  const expanded = terminalTurnExpandedForDisplay(turn)
    || options.expandedEntryKeys?.has(terminalTurnExpansionEntryKey(turn.id)) === true
  const cancelled = turn.kernelSettlementStatus === "cancelled"
  const failed = turn.kernelSettlementStatus === "failed"
  const summary = completed && !cancelled
    ? failed
      ? renderableBodyEntries.slice().reverse().find((entry) => entry.role === "error" && entry.text.trim().length > 0) ?? null
      : latestAssistantSummaryEntry(renderableBodyEntries)
    : null
  const cancellationOutcome = (
    completed
    && !internalContinuation
    && cancelled
  )
  const visibleBodyEntries = summary
    ? renderableBodyEntries.filter((entry) => entry !== summary)
    : renderableBodyEntries
  const settledBodyEntries = foldTerminalToolSnapshots(visibleBodyEntries.filter((entry) => entry.role !== "status"))
    .map((entry) => terminalizeRunningToolEntry(turn, entry))
  const shouldRenderMeta = !internalContinuation && (Boolean(prompt) || (completed && segmentKey === turn.id))
  const activitySummary = completed ? terminalTurnActivitySummary(settledBodyEntries) : ""
  return `
    <section class="freeform-turn" data-render-key="turn:${options.escapeHtml(segmentKey)}" data-turn-key="${options.escapeHtml(turn.id)}" data-expanded="${expanded ? "true" : "false"}">
      ${prompt ? options.renderEntry(prompt) : ""}
      ${shouldRenderMeta ? renderFreeformTurnMeta(turn, prompt ?? null, options, segmentKey, expanded, activitySummary) : ""}
      ${renderTerminalTurnBody(settledBodyEntries, expanded, options, segmentKey)}
      ${summary ? renderFreeformTurnSummary(summary, options) : ""}
      ${taskNotices.map((entry) => options.renderEntry(entry)).join("")}
      ${cancellationOutcome ? renderCancelledTurnOutcome(options) : ""}
    </section>
  `
}

export function terminalOutputEntryIsSteeringPrompt(entry: TerminalOutputEntry): boolean {
  return entry.role === "user" && entry.promptRole === "steering"
}

function terminalOutputEntryIsPrivateContinuationPrompt(entry: TerminalOutputEntry): boolean {
  return terminalPromptTextIsPrivateContinuation(entry.text)
}

function renderTerminalTurnBody(
  entries: readonly TerminalOutputEntry[],
  expanded: boolean,
  options: TerminalTurnRenderOptions,
  segmentKey: string,
): string {
  const collapsed = entries.length === 0 || !expanded
  return `
    <div class="freeform-turn-body" data-render-key="turn-body:${options.escapeHtml(segmentKey)}" data-collapsed="${collapsed ? "true" : "false"}" data-empty="${entries.length === 0 ? "true" : "false"}" aria-hidden="${collapsed ? "true" : "false"}">
      ${entries.map((entry) => options.renderEntry(entry)).join("")}
    </div>
  `
}

// The row a full render draws for this tool entry, or null when another row
// of its turn absorbs it — lets one tool row be patched without a full render.
export function terminalRenderedToolRow(
  entries: readonly TerminalOutputEntry[],
  key: string,
): TerminalOutputEntry | null {
  const target = entries.find((entry) => entry.key === key)
  if (!target || target.role !== "tool") {
    return null
  }
  const turnTools = entries.filter((entry) => entry.turnId === target.turnId)
  return foldTerminalToolSnapshots(turnTools).find((entry) => entry.key === key) ?? null
}

// One row per tool call, in output order. Later snapshots of the same call
// (history keeps one row per status change) update the call's first row; a
// call that runs again after it finished is a new row.
export function foldTerminalToolSnapshots(entries: readonly TerminalOutputEntry[]): TerminalOutputEntry[] {
  const folded: TerminalOutputEntry[] = []
  const indexByIdentity = new Map<string, number>()
  for (const entry of entries) {
    if (entry.role !== "tool") {
      for (const identity of indexByIdentity.keys()) {
        if (identity.startsWith("command:")) indexByIdentity.delete(identity)
      }
    }
    const candidates = entry.role === "tool" && !entry.historyBlob
      ? terminalToolSnapshotIdentities(entry.sourceText ?? entry.text)
      : []
    const strong = candidates.filter((identity) => !identity.startsWith("command:"))
    const identities = strong.length ? strong : candidates
    const existingIndex = identities.map((identity) => indexByIdentity.get(identity)).find((index) => index !== undefined)
    const existing = existingIndex === undefined ? undefined : folded[existingIndex]
    if (
      existingIndex === undefined
      || !existing
      || (isTerminalToolSnapshot(existing) && (isRunningToolSnapshot(entry) || distinctToolCallIds(existing, entry)))
    ) {
      for (const identity of identities) {
        indexByIdentity.set(identity, folded.length)
      }
      folded.push(entry)
      continue
    }
    const mergedSnapshot = mergeTerminalToolCallAndOutputSnapshots(
      existing.sourceText ?? existing.text,
      entry.sourceText ?? entry.text,
    )
    for (const identity of identities) {
      indexByIdentity.set(identity, existingIndex)
    }
    folded[existingIndex] = mergedSnapshot
      ? { ...entry, text: mergedSnapshot, sourceText: mergedSnapshot }
      : entry
  }
  return folded
}

// After a call finished, a snapshot with another id is another call even when
// the command matches; snapshots of one process (polls) stay one row.
function distinctToolCallIds(left: TerminalOutputEntry, right: TerminalOutputEntry): boolean {
  const leftText = left.sourceText ?? left.text
  const rightText = right.sourceText ?? right.text
  const leftId = terminalToolCallId(leftText)
  const rightId = terminalToolCallId(rightText)
  if (leftId === null || rightId === null || leftId === rightId) {
    return false
  }
  const leftProcesses = terminalToolSnapshotIdentities(leftText).filter((identity) => identity.startsWith("process:"))
  return !terminalToolSnapshotIdentities(rightText).some((identity) => leftProcesses.includes(identity))
}

function isRunningToolSnapshot(entry: TerminalOutputEntry): boolean {
  return terminalToolEntrySnapshotStatus(entry) === "running"
}

function isTerminalToolSnapshot(entry: TerminalOutputEntry): boolean {
  const status = terminalToolEntrySnapshotStatus(entry)
  return status === "completed" || status === "error" || status === "cancelled" || status === "ended"
}

function terminalizeRunningToolEntry(
  turn: TerminalTurnState,
  entry: TerminalOutputEntry,
): TerminalOutputEntry {
  if (entry.role !== "tool" || !terminalTurnIsCompleted(turn)) {
    return entry
  }
  if (entry.historyBlob?.endedTitle) {
    return { ...entry, historyBlob: { ...entry.historyBlob, title: entry.historyBlob.endedTitle } }
  }
  if (entry.historyBlob && /\bRUNNING\b/.test(entry.historyBlob.title)) {
    return {
      ...entry,
      historyBlob: {
        ...entry.historyBlob,
        title: entry.historyBlob.title.replace(/\bRUNNING\b/, "ENDED"),
      },
    }
  }
  if (terminalToolEntrySnapshotStatus(entry) !== "running") {
    return entry
  }
  const text = toolSnapshotWithStatus(entry.text, "ended")
  const sourceText = entry.sourceText === undefined
    ? undefined
    : toolSnapshotWithStatus(entry.sourceText, "ended")
  return {
    ...entry,
    text,
    ...(sourceText === undefined ? {} : { sourceText }),
  }
}

function terminalToolEntrySnapshotStatus(entry: TerminalOutputEntry): string | null {
  return terminalToolSnapshotStatus(entry.sourceText ?? "")
    ?? terminalToolSnapshotStatus(entry.text)
}

function toolSnapshotWithStatus(text: string, status: string): string {
  try {
    const snapshot = JSON.parse(text) as Record<string, unknown>
    if (!snapshot || typeof snapshot !== "object" || Array.isArray(snapshot)) {
      return text
    }
    return JSON.stringify({ ...snapshot, status })
  } catch {
    return text
  }
}

export function latestAssistantSummaryEntry(entries: TerminalOutputEntry[]): TerminalOutputEntry | null {
  const summaryEntries = entries.filter((entry) => (
    (entry.role === "assistant" || entry.role === "error")
    && entry.text.trim().length > 0
  ))
  const authoritativeHistorySummaries = summaryEntries.filter((entry) => entry.historyTurnSummary)
  const candidate = authoritativeHistorySummaries.at(-1) ?? summaryEntries.at(-1) ?? null
  if (!candidate) {
    return null
  }
  // A later tool call makes the message intermediate. A late thought stays
  // inside the fold without hiding the final answer.
  const followedByActivity = entries
    .slice(entries.indexOf(candidate) + 1)
    .some((entry) => entry.role === "tool")
  return followedByActivity ? null : candidate
}

export function renderFreeformTurnMeta(
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
  options: Pick<TerminalTurnRenderOptions, "completedTurnForAgent" | "escapeHtml" | "formatDateTime" | "turnActions" | "turns">,
  segmentKey = turn.id,
  expandedOverride?: boolean,
  activitySummary = "",
): string {
  const externalMarker = externalTurnMarker(turn, prompt)
  const completed = terminalTurnIsCompleted(turn)
  const expanded = expandedOverride ?? terminalTurnExpandedForDisplay(turn)
  const tooltipText = formatTurnMetaTooltipText(turn, externalMarker, options)
  const tooltip = prompt
    ? `<span class="ds-tooltip freeform-turn-meta-tooltip" role="tooltip">${options.escapeHtml(tooltipText)}</span>`
    : ""
  return `
    <div class="freeform-turn-meta-row freeform-turn-start-row" data-render-key="turn-meta-row:${options.escapeHtml(segmentKey)}">
      <div class="freeform-turn-meta" data-render-key="turn-meta:${options.escapeHtml(segmentKey)}">
        ${renderTurnClockIcon()}
        <span class="ds-tooltip-host freeform-turn-meta-main" data-tooltip-delay="slow" data-tooltip-trigger="hover">
          <span class="freeform-turn-meta-text">${options.escapeHtml(formatTurnStartMetaText(turn, options))}</span>
          ${tooltip}
        </span>
        ${externalMarker ? renderExternalTurnMarker(externalMarker, options.escapeHtml) : ""}
        ${completed ? `<span class="freeform-turn-meta-text">${options.escapeHtml(formatTurnCompletionMetaText(turn))}</span>` : ""}
        ${activitySummary ? `<span class="freeform-turn-meta-text freeform-turn-activity" data-turn-activity>${options.escapeHtml(activitySummary)}</span>` : ""}
        ${completed && options.turnActions !== false ? renderFreeformTurnActions(turn, prompt, options) : ""}
      </div>
      ${completed ? renderFreeformTurnChevron(true, expanded ? "Collapse turn" : "Expand turn", options.escapeHtml, expanded) : ""}
    </div>
  `
}

function formatTurnMetaTooltipText(
  turn: TerminalTurnState,
  marker: ExternalProviderObservedTurnMarker | null,
  options: Pick<TerminalTurnRenderOptions, "formatDateTime"> = {},
): string {
  const parts = [formatTurnStartMetaText(turn, options)]
  if (marker) {
    parts.push(`external ${marker.provider} session ID: ${marker.providerSessionId}`)
  }
  if (terminalTurnIsCompleted(turn)) {
    parts.push(formatTurnCompletionMetaText(turn))
  }
  return parts.join(", ")
}

function renderFreeformTurnActions(
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
  options: Pick<TerminalTurnRenderOptions, "completedTurnForAgent" | "escapeHtml" | "turns">,
): string {
  const latest = options.completedTurnForAgent?.(turn.agentId) ?? null
  const externalIdentityConflict = latest
    ? completedTurnExternalIdentityConflictsTurn(latest, turn, prompt)
    : false
  const isLatestCompletedTurn = latest
    ? completedTurnActionMatchesTurn(latest, turn, prompt)
      || (!externalIdentityConflict && isLatestRenderedCompletedTurn(turn, options.turns))
    : false
  const undo = isLatestCompletedTurn && latest
    ? renderFreeformTurnUndoAction(turn, latest, options)
    : renderFreeformTurnHistoricalUndoAction(turn, prompt, options)
  return `
    <span class="freeform-turn-actions" data-freeform-turn-actions data-turn-id="${options.escapeHtml(turn.id)}">
      ${undo}
      <button class="freeform-turn-action freeform-turn-fork" type="button" data-freeform-turn-action="fork" data-freeform-agent-fork data-agent-id="${options.escapeHtml(turn.agentId)}" data-turn-id="${options.escapeHtml(turn.id)}" aria-label="Fork agent" title="Fork agent">
        <span>Fork</span>
        ${renderForkIcon()}
      </button>
    </span>
  `
}

function renderFreeformTurnUndoAction(
  turn: TerminalTurnState,
  latest: TerminalTurnCompletedAction,
  options: Pick<TerminalTurnRenderOptions, "escapeHtml">,
): string {
  const disabled = latest.undoAvailable !== true || turn.undoAccepted === true
  const title = disabled
    ? turn.undoAccepted === true
      ? "turn already undone"
      : latest.undoUnavailableReason?.trim() || "Undo is unavailable for this turn"
    : "Undo turn"
  return `
    <button class="freeform-turn-action freeform-turn-undo" type="button" data-freeform-turn-action="undo" data-freeform-turn-undo data-agent-id="${options.escapeHtml(turn.agentId)}" data-turn-id="${options.escapeHtml(latest.turnId)}" title="${options.escapeHtml(title)}" ${disabled ? "disabled" : ""}>
      <span>Undo</span>
      ${renderUndoIcon()}
    </button>
  `
}

function renderFreeformTurnHistoricalUndoAction(
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
  options: Pick<TerminalTurnRenderOptions, "escapeHtml">,
): string {
  const turnRef = historicalUndoTurnRef(turn, prompt)
  const disabled = turn.undoAccepted === true || !turnRef
  const title = turn.undoAccepted === true
    ? "turn already undone"
    : turnRef
      ? "Undo turn"
      : "Undo is unavailable for this turn"
  return `
    <button class="freeform-turn-action freeform-turn-undo" type="button" data-freeform-turn-action="undo" data-freeform-turn-undo data-agent-id="${options.escapeHtml(turn.agentId)}" data-turn-id="${options.escapeHtml(turnRef ?? turn.id)}" title="${options.escapeHtml(title)}" ${disabled ? "disabled" : ""}>
      <span>Undo</span>
      ${renderUndoIcon()}
    </button>
  `
}

function historicalUndoTurnRef(turn: TerminalTurnState, prompt: TerminalOutputEntry | null): string | null {
  return turn.kernelPromptId?.trim()
    || turn.id.trim()
    || prompt?.key.trim()
    || null
}

function completedTurnActionMatchesTurn(
  latest: TerminalTurnCompletedAction,
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
): boolean {
  if (completedTurnExternalIdentityConflictsTurn(latest, turn, prompt)) {
    return false
  }
  return latest.turnId === turn.id
    || latest.promptId === turn.id
    || latest.turnId === turn.kernelPromptId
    || latest.promptId === turn.kernelPromptId
    || latest.promptId === prompt?.key
    || (Boolean(latest.providerRunId) && latest.providerRunId === turn.kernelProviderRunId)
    || (latest.completedAtMs !== undefined && latest.completedAtMs === turn.completedAtMs)
}

function completedTurnExternalIdentityConflictsTurn(
  latest: TerminalTurnCompletedAction,
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
): boolean {
  const latestIdentity = completedTurnExternalIdentityFields(latest)
  const turnIdentity = turnExternalIdentityFields(turn, prompt)
  return externalProviderObservedExactIdentityConflicts(latestIdentity, turnIdentity)
}

function completedTurnExternalIdentityFields(
  turn: TerminalTurnCompletedAction,
): ExternalProviderObservedIdentityFields {
  return {
    ...(turn.externalProvider !== undefined ? { externalProvider: turn.externalProvider } : {}),
    ...(turn.externalProviderSessionId !== undefined
      ? { externalProviderSessionId: turn.externalProviderSessionId }
      : {}),
    ...(turn.externalProviderTurnId !== undefined ? { externalProviderTurnId: turn.externalProviderTurnId } : {}),
  }
}

function turnExternalIdentityFields(
  turn: TerminalTurnState,
  prompt: TerminalOutputEntry | null,
): ExternalProviderObservedIdentityFields {
  const promptMetadata = prompt ? externalProviderObservedEntryTurnMetadata(prompt) : null
  return {
    ...(turn.kernelExternalProvider !== undefined
      ? { externalProvider: turn.kernelExternalProvider }
      : promptMetadata?.externalProvider !== undefined
        ? { externalProvider: promptMetadata.externalProvider }
        : {}),
    ...(turn.kernelExternalProviderSessionId !== undefined
      ? { externalProviderSessionId: turn.kernelExternalProviderSessionId }
      : promptMetadata?.externalProviderSessionId !== undefined
        ? { externalProviderSessionId: promptMetadata.externalProviderSessionId }
        : {}),
    ...(turn.kernelExternalProviderTurnId !== undefined
      ? { externalProviderTurnId: turn.kernelExternalProviderTurnId }
      : promptMetadata?.externalProviderTurnId !== undefined
        ? { externalProviderTurnId: promptMetadata.externalProviderTurnId }
        : {}),
  }
}

function isLatestRenderedCompletedTurn(
  turn: TerminalTurnState,
  turns: ReadonlyMap<string, TerminalTurnState>,
): boolean {
  if (!terminalTurnIsCompleted(turn)) {
    return false
  }
  let latest: TerminalTurnState | null = null
  for (const candidate of turns.values()) {
    if (candidate.agentId !== turn.agentId || !terminalTurnIsCompleted(candidate)) {
      continue
    }
    if (!latest || terminalTurnCompletionSortValue(candidate) > terminalTurnCompletionSortValue(latest)) {
      latest = candidate
    }
  }
  return latest?.id === turn.id
}

export function renderFreeformTurnSummary(
  entry: TerminalOutputEntry,
  options: Pick<TerminalTurnRenderOptions, "escapeHtml" | "renderTranscriptHtml">,
): string {
  const text = entry.text
  const errorClass = entry.role === "error" ? " freeform-error-output" : ""
  return `<section class="freeform-message freeform-turn-summary freeform-agent-output${errorClass}" data-render-key="turn-summary:${options.escapeHtml(entry.key)}" data-turn-summary>${options.renderTranscriptHtml(text)}</section>`
}

function externalTurnMarker(turn: TerminalTurnState, prompt: TerminalOutputEntry | null): ExternalProviderObservedTurnMarker | null {
  if (turn.kernelPromptOrigin && !terminalTurnPromptOriginIsExternal(turn)) {
    return null
  }
  if (!turn.kernelPromptOrigin && prompt?.promptOrigin && !terminalPromptOriginIsExternal(prompt.promptOrigin)) {
    return null
  }
  const turnMarker = externalProviderObservedTurnIdentityMarker(turn)
  if (turnMarker) {
    return turnMarker
  }
  return externalProviderObservedTurnMarker(prompt ? [prompt] : [])
}

function externalProviderObservedTurnIdentityMarker(
  turn: TerminalTurnState,
): ExternalProviderObservedTurnMarker | null {
  const provider = turn.kernelExternalProvider?.trim()
  const providerSessionId = turn.kernelExternalProviderSessionId?.trim()
  if (provider && providerSessionId) {
    return { provider, providerSessionId }
  }
  return null
}

function renderExternalTurnMarker(marker: ExternalProviderObservedTurnMarker, escapeHtml: (value: string) => string): string {
  const title = `${marker.provider} session ID: ${marker.providerSessionId}`
  return `
    <span class="freeform-turn-external-marker">
      <span class="freeform-turn-external-label">external</span>
      <span class="ds-tooltip-host freeform-turn-external-info" data-tooltip-delay="slow" tabindex="0" aria-label="${escapeHtml(title)}">
        ${renderInfoIcon()}
        <span class="ds-tooltip" role="tooltip">${escapeHtml(title)}</span>
      </span>
    </span>
  `
}

export function formatTurnMetaText(
  turn: TerminalTurnState,
  options: Pick<TerminalTurnRenderOptions, "formatDateTime"> = {},
): string {
  return !terminalTurnIsCompleted(turn)
    ? formatTurnStartMetaText(turn, options)
    : `${formatTurnStartMetaText(turn, options)}, ${formatTurnCompletionMetaText(turn)}`
}

export function formatTurnStartMetaText(
  turn: TerminalTurnState,
  options: Pick<TerminalTurnRenderOptions, "formatDateTime"> = {},
): string {
  if (!terminalTurnIsCompleted(turn)) {
    return `sent at ${formatSystemDateTime(turn.startedAtMs, options)}, running`
  }
  return `sent at ${formatSystemDateTime(turn.startedAtMs, options)}`
}

export function formatTurnCompletionMetaText(turn: TerminalTurnState): string {
  if (!terminalTurnIsCompleted(turn)) {
    return "running"
  }
  if (turn.kernelSettlementStatus === "cancelled") {
    return `cancelled after ${formatTurnDuration(turn.startedAtMs, turn.completedAtMs)}`
  }
  if (turn.kernelSettlementStatus === "failed") {
    return `failed after ${formatTurnDuration(turn.startedAtMs, turn.completedAtMs)}`
  }
  return `worked for ${formatTurnDuration(turn.startedAtMs, turn.completedAtMs)}`
}

function renderCancelledTurnOutcome(
  options: Pick<TerminalTurnRenderOptions, "escapeHtml" | "renderTranscriptHtml">,
): string {
  return `<section class="freeform-message freeform-turn-summary freeform-agent-output freeform-error-output" data-render-key="turn-summary:cancelled" data-turn-summary>${options.renderTranscriptHtml("Turn cancelled.")}</section>`
}

export function formatSystemDateTime(
  timestampMs: number,
  options: Pick<TerminalTurnRenderOptions, "formatDateTime"> = {},
): string {
  return options.formatDateTime?.(timestampMs) ?? formatLocalInstant(timestampMs, {
    dateStyle: "short",
    timeStyle: "medium",
  }) ?? ""
}

export function formatTurnDuration(startedAtMs: number, completedAtMs: number): string {
  const durationMs = Math.max(0, completedAtMs - startedAtMs)
  return formatDurationMs(durationMs)
}

export function formatElapsedTurnDuration(startedAtMs: number, nowMs: number): string {
  return formatDurationMs(Math.max(0, nowMs - startedAtMs))
}

export function formatDurationMs(durationMs: number): string {
  if (durationMs > 0 && durationMs < 1_000) {
    return "<1s"
  }
  const totalSeconds = Math.floor(durationMs / 1000)
  if (totalSeconds < 60) {
    return `${totalSeconds}s`
  }
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, "0")}m ${String(seconds).padStart(2, "0")}s`
  }
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`
}

function renderTurnClockIcon(): string {
  return `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>`
}

function renderInfoIcon(): string {
  return `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9" /><path d="M12 11v5" /><path d="M12 8h.01" /></svg>`
}

function renderFreeformTurnChevron(
  enabled: boolean,
  ariaLabel: string,
  escapeHtml: (value: string) => string,
  expanded = false,
): string {
  return `
    <button class="freeform-turn-chevron-button" type="button" ${enabled ? "data-freeform-turn-toggle" : ""} aria-expanded="${expanded ? "true" : "false"}" aria-label="${escapeHtml(ariaLabel)}" ${enabled ? "" : "disabled"}>
      <span class="freeform-turn-chevron" aria-hidden="true">${enabled ? renderFreeformChevronIcon() : "..."}</span>
    </button>
  `
}

function renderUndoIcon(): string {
  return `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14 4 9l5-5" /><path d="M4 9h10a6 6 0 1 1 0 12h-1" /></svg>`
}

function renderForkIcon(): string {
  return `<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="6" r="2" /><circle cx="6" cy="18" r="2" /><circle cx="18" cy="18" r="2" /><path d="M12 8v4" /><path d="M12 12H8a2 2 0 0 0-2 2v2" /><path d="M12 12h4a2 2 0 0 1 2 2v2" /></svg>`
}
