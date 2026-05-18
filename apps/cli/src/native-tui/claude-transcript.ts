import { readFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"

export type ClaudeHookEvent = {
  index: number
  hook_event_name: string
  hook_context_request_id?: string | null
  prompt?: string | null
  transcript_path?: string | null
  permission_mode?: string | null
  tool_name?: string | null
  tool_input?: unknown
  tool_response?: unknown
  error?: unknown
}

export async function readClaudeHookEvents(file: string): Promise<ClaudeHookEvent[]> {
  const raw = await readFile(file, "utf8").catch(() => "")
  return raw
    .split("\n")
    .map((line, index) => ({ line, index }))
    .filter(({ line }) => line.trim())
    .map(({ line, index }) => {
      try {
        const value = JSON.parse(line) as Omit<ClaudeHookEvent, "index">
        return { ...value, index }
      } catch {
        return { index, hook_event_name: "parse_error" }
      }
    })
}

export async function drainAssistantText(transcriptPath: string, offsets: Map<string, number>): Promise<string> {
  const { text, lineCount } = await readAssistantTextAfterOffset(transcriptPath, offsets.get(transcriptPath) ?? 0)
  offsets.set(transcriptPath, lineCount)
  return text
}

export async function waitForAssistantText(transcriptPath: string, offsets: Map<string, number>): Promise<string> {
  const start = offsets.get(transcriptPath) ?? 0
  const deadline = Date.now() + 5_000
  let latestLineCount = start
  while (Date.now() < deadline) {
    const { text, lineCount } = await readAssistantTextAfterOffset(transcriptPath, start)
    latestLineCount = Math.max(latestLineCount, lineCount)
    if (text.trim()) {
      offsets.set(transcriptPath, lineCount)
      return text
    }
    await sleep(200)
  }
  offsets.set(transcriptPath, latestLineCount)
  return ""
}

async function readAssistantTextAfterOffset(transcriptPath: string, start: number): Promise<{ text: string; lineCount: number }> {
  const raw = await readFile(transcriptPath, "utf8").catch(() => "")
  const lines = raw.split("\n").filter((line) => line.trim())
  const texts: string[] = []
  for (const line of lines.slice(start)) {
    try {
      const entry = JSON.parse(line)
      if (isAssistantTranscriptEntry(entry)) {
        const text = collectTextValues(entry).join("\n").trim()
        if (text) texts.push(text)
      }
    } catch {}
  }
  return { text: texts.join("\n"), lineCount: lines.length }
}

function isAssistantTranscriptEntry(value: unknown): boolean {
  if (!value || typeof value !== "object") return false
  const record = value as Record<string, unknown>
  if (record.type === "assistant" || record.role === "assistant") return true
  const message = record.message
  return Boolean(message && typeof message === "object" && (message as Record<string, unknown>).role === "assistant")
}

function collectTextValues(value: unknown): string[] {
  if (!value || typeof value !== "object") return []
  if (Array.isArray(value)) return value.flatMap((entry) => collectTextValues(entry))
  const record = value as Record<string, unknown>
  const text = typeof record.text === "string" ? [record.text] : []
  return text.concat(Object.values(record).flatMap((entry) => collectTextValues(entry)))
}
