/** What the kernel says about one compaction: the text of a `context_compacted` entry, replaced as it runs, finishes or fails. */
export type ContextCompaction =
  | { readonly state: "running"; readonly started_at_ms: number }
  | { readonly state: "done"; readonly summary: string }
  | { readonly state: "failed"; readonly reason: string }

export function parseContextCompaction(text: string): ContextCompaction {
  const value: unknown = JSON.parse(text)
  if (value && typeof value === "object") {
    const fields = value as Record<string, unknown>
    if (fields.state === "running" && typeof fields.started_at_ms === "number") {
      return { state: "running", started_at_ms: fields.started_at_ms }
    }
    if (fields.state === "done" && typeof fields.summary === "string") {
      return { state: "done", summary: fields.summary }
    }
    if (fields.state === "failed" && typeof fields.reason === "string") {
      return { state: "failed", reason: fields.reason }
    }
  }
  throw new Error(`the kernel sent a compaction this client does not know: ${text.slice(0, 120)}`)
}
