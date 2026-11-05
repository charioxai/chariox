export function terminalToolCallKeyPart(text: string): string | null {
  const id = terminalToolCallId(text)
  return id ? encodeURIComponent(id) : null
}

export function terminalToolSnapshotIdentity(text: string): string | null {
  return terminalToolSnapshotIdentities(text)[0] ?? null
}

export function terminalToolSnapshotIdentities(text: string): string[] {
  const update = parseToolSnapshot(text)
  if (!update) {
    return []
  }
  const tool = update.tool ?? "tool"
  const identities: string[] = []
  const processId = readToolProcessId(update.raw)
  if (processId) {
    identities.push(`process:${tool}:${processId}`)
  }
  if (update.id) {
    identities.push(`id:${tool}:${update.id}`)
  }
  if (update.callId) {
    identities.push(`call:${update.callId}`)
    identities.push(`call:${tool}:${update.callId}`)
  }
  const command = readToolInputString(update.input, "command")
  if (command) {
    const cwd = readToolInputString(update.input, "cwd") ?? ""
    identities.push(`command:${tool}:${cwd}:${command}`)
  }
  return identities
}

export function terminalToolSnapshotStatus(text: string): string | null {
  const update = parseToolSnapshot(text)
  return update?.status ?? null
}

export function terminalToolCallId(text: string): string | null {
  return parseToolSnapshot(text)?.id ?? null
}

export function mergeTerminalToolCallAndOutputSnapshots(
  previousText: string,
  nextText: string,
): string | null {
  const previous = parseToolSnapshotRecord(previousText)
  const next = parseToolSnapshotRecord(nextText)
  const previousCallId = previous ? readNonEmptyString(previous.call_id) : null
  const nextCallId = next ? readNonEmptyString(next.call_id) : null
  if (!previous || !next || !previousCallId || previousCallId !== nextCallId) {
    return null
  }
  const previousIsOutput = toolSnapshotRecordIsOutput(previous)
  const nextIsOutput = toolSnapshotRecordIsOutput(next)
  const previousHasResult = toolSnapshotRecordHasResult(previous)
  const nextHasResult = toolSnapshotRecordHasResult(next)
  let invocation: Record<string, unknown>
  let result: Record<string, unknown>
  if (previousIsOutput !== nextIsOutput) {
    invocation = previousIsOutput ? next : previous
    result = previousIsOutput ? previous : next
  } else if (previousHasResult !== nextHasResult) {
    invocation = previousHasResult ? next : previous
    result = previousHasResult ? previous : next
  } else {
    return null
  }
  const id = readNonEmptyString(invocation.id)
    ?? readNonEmptyString(result.id)
    ?? previousCallId
  const tool = readNonEmptyString(invocation.tool)
    ?? readNonEmptyString(invocation.name)
    ?? readNonEmptyString(result.tool)
  const type = readNonEmptyString(invocation.type)
  const input = invocation.input ?? invocation.arguments
  const status = readNonEmptyString(result.status) ?? (hasOwn(result, "error") ? "error" : "completed")
  const merged: Record<string, unknown> = {
    ...invocation,
    ...result,
    id,
    call_id: previousCallId,
    status,
  }
  if (tool) {
    merged.tool = tool
  }
  if (type) {
    merged.type = type
  }
  if (input !== undefined) {
    merged.input = input
  }
  return JSON.stringify(merged)
}

function parseToolSnapshot(text: string): {
  readonly id: string | null
  readonly callId: string | null
  readonly tool: string | null
  readonly status: string | null
  readonly input: unknown
  readonly raw: string | null
} | null {
  const parsed = parseToolSnapshotRecord(text)
  if (!parsed) {
    return null
  }
  return {
    id: readNonEmptyString(parsed.id),
    callId: readNonEmptyString(parsed.call_id),
    tool: readNonEmptyString(parsed.tool),
    status: readNonEmptyString(parsed.status),
    input: parsed.input,
    raw: readNonEmptyString(parsed.raw),
  }
}

function parseToolSnapshotRecord(text: string): Record<string, unknown> | null {
  const jsonStart = text.indexOf("{")
  const candidates = jsonStart > 0 ? [text, text.slice(jsonStart)] : [text]
  for (const candidate of candidates) {
    try {
      const parsed = JSON.parse(candidate) as unknown
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>
      }
    } catch {
      // Try the provider-labelled JSON payload after the text prefix.
    }
  }
  return null
}

function toolSnapshotRecordIsOutput(record: Record<string, unknown>): boolean {
  const type = readNonEmptyString(record.type)?.toLowerCase()
  const tool = readNonEmptyString(record.tool)?.toLowerCase()
  return type?.endsWith("_output") === true
    || tool === "function_call_output"
    || (hasOwn(record, "output") && !hasOwn(record, "input") && !hasOwn(record, "arguments"))
}

function toolSnapshotRecordHasResult(record: Record<string, unknown>): boolean {
  return hasOwn(record, "output") || hasOwn(record, "error")
}

function readToolProcessId(raw: string | null): string | null {
  const match = raw?.match(/\bprocess_id:\s*(\d+)\b/)
  return match?.[1] ?? null
}

function readToolInputString(input: unknown, field: string): string | null {
  if (!input || typeof input !== "object") {
    return null
  }
  return readNonEmptyString((input as Record<string, unknown>)[field])
}

function readNonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null
}

function hasOwn(record: Record<string, unknown>, field: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, field)
}
