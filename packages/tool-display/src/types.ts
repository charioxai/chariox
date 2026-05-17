export type ToolTranscriptUpdate = {
  id: string
  tool?: string
  status?: string
  title?: string
  description?: string
  text?: string
  input?: unknown
  output?: string
  error?: string
  raw?: string
}

export type InlineCodeSpan = {
  text: string
  code: boolean
}

export type ToolDisplayStatus = "running" | "completed" | "error" | "cancelled" | string

export type ToolDisplayBlock =
  | { kind: "text"; text: string }
  | { kind: "code"; language: string; text: string }
  | { kind: "patch"; files: ToolDisplayPatchFile[] }
  | { kind: "json"; value: unknown }
  | { kind: "table"; columns: string[]; rows: string[][] }

export type ApplyPatchFile = {
  kind: "add" | "delete" | "update" | "move"
  filePath: string
  title: string
  diff: string | null
}

export type ToolDisplayPatchLine = {
  kind: "context" | "added" | "removed" | "meta"
  text: string
}

export type ToolDisplayPatchFile = ApplyPatchFile & {
  previewLines: ToolDisplayPatchLine[]
}

export type ToolDisplay = {
  version: 1
  tool: string
  status?: ToolDisplayStatus | undefined
  title: string
  summary: string
  collapsed: {
    title: string
    summary: string
  }
  blocks: ToolDisplayBlock[]
}
