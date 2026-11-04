import type {
  EnvironmentEntryRef,
  EnvironmentOperationView,
  ProjectEnvironmentFile,
  ProjectEnvironmentSecret,
  ProjectEnvironmentView,
  SetupState,
} from "@chariox/kernel-client/kernel-types"

export type EnvironmentFocus =
  | { readonly kind: "run" }
  | { readonly kind: "update" }
  | { readonly kind: "draft" }
  | { readonly kind: "check" }
  | { readonly kind: "mcp"; readonly index: number }
  | { readonly kind: "skill"; readonly index: number }
  | { readonly kind: "proposal"; readonly index: number }
  | { readonly kind: "script" }
  | { readonly kind: "variables" }
  | { readonly kind: "secret"; readonly index: number }
  | { readonly kind: "addSecret" }
  | { readonly kind: "file"; readonly index: number }
  | { readonly kind: "addFile" }

export type EnvironmentTextField = "script" | "variables"

/** The input that currently owns the keyboard. A secret value is represented by its length only. */
export type EnvironmentInputView =
  | { readonly kind: "text"; readonly field: EnvironmentTextField; readonly text: string; readonly cursor: number }
  | { readonly kind: "line"; readonly target: "secretName" | "filePath"; readonly text: string }
  | { readonly kind: "secretValue"; readonly name: string; readonly length: number }

/** Another kernel of the same user that an Import can take entries from. */
export type EnvironmentSourceKernel = { readonly id: string; readonly label: string }

export type EnvironmentDialogModel = {
  readonly loading: boolean
  /** Other kernels of this user; empty when there is none, which hides Update and Import here. */
  readonly sources: readonly EnvironmentSourceKernel[]
  readonly sourceIndex: number
  /** The last Import this dialog started (Import here or Update). */
  readonly transfer: EnvironmentOperationView | null
  /** The last Draft or Check with agent this dialog started. */
  readonly draft: EnvironmentOperationView | null
  readonly environment: ProjectEnvironmentView | null
  readonly script: string
  readonly variables: string
  readonly focus: EnvironmentFocus
  readonly input: EnvironmentInputView | null
  readonly operation: EnvironmentOperationView | null
  readonly message: string | null
  readonly nowMs: number
}

export type EnvironmentRowTone = "text" | "muted" | "warning" | "error"

export type EnvironmentDialogRow = {
  readonly text: string
  readonly tone: EnvironmentRowTone
  readonly focused: boolean
  readonly bold: boolean
}

export type EnvironmentDialogView = {
  readonly hint: string
  readonly rows: readonly EnvironmentDialogRow[]
  readonly message: string | null
}

const FIELD_LINES = 6
const OUTPUT_LINES = 8
const CURSOR = "▏"
const MASK = "•"

export function focusKey(focus: EnvironmentFocus): string {
  return "index" in focus ? `${focus.kind}:${focus.index}` : focus.kind
}

/** Every row the keyboard can select, in screen order. */
export function environmentFocusables(environment: ProjectEnvironmentView | null, hasSources = false): EnvironmentFocus[] {
  if (!environment) return []
  return [
    { kind: "run" },
    ...(hasSources && environment.exists ? [{ kind: "update" } as const] : []),
    { kind: "script" },
    ...(environment.exists ? [{ kind: "check" } as const] : [{ kind: "draft" } as const]),
    { kind: "variables" },
    ...environment.secrets.map((_, index): EnvironmentFocus => ({ kind: "secret", index })),
    { kind: "addSecret" },
    ...environment.files.map((_, index): EnvironmentFocus => ({ kind: "file", index })),
    { kind: "addFile" },
    ...environment.mcps.map((_, index): EnvironmentFocus => ({ kind: "mcp", index })),
    ...environment.skills.map((_, index): EnvironmentFocus => ({ kind: "skill", index })),
    ...environment.proposals.map((_, index): EnvironmentFocus => ({ kind: "proposal", index })),
  ]
}

/** "Updating from Miguel's Mac · 2 of 4" while an Import runs, otherwise null. */
export function updatingText(transfer: EnvironmentOperationView | null, chosen: string | null): string | null {
  if (transfer?.kind !== "import" || transfer.state !== "running") return null
  const finished = transfer.entries.filter((entry) => entry.state === "imported" || entry.state === "failed").length
  const source = transfer.sourceLabel ?? chosen ?? "another machine"
  return transfer.entries.length === 0 ? `Updating from ${source}` : `Updating from ${source} · ${finished} of ${transfer.entries.length}`
}

/** What a row says about the last Import: where it comes from or came from, or why it did not arrive; null when it did not touch the row. */
export function importRowText(transfer: EnvironmentOperationView | null, entry: EnvironmentEntryRef, chosen: string | null): { readonly text: string; readonly problem: boolean } | null {
  const found = transfer?.kind === "import" ? transfer.entries.find((candidate) => candidate.kind === entry.kind && candidate.name === entry.name) : undefined
  if (!transfer || !found) return null
  const source = transfer.sourceLabel ?? chosen ?? "another machine"
  switch (found.state) {
    case "pending": case "importing": return { text: `Importing from ${source}`, problem: false }
    case "imported": return { text: `Imported from ${source}`, problem: false }
    case "failed": return { text: found.reason ?? "Import failed", problem: true }
  }
}

export function relativeAge(thenMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - thenMs) / 1000))
  if (seconds < 60) return "just now"
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  return `${Math.floor(hours / 24)} d ago`
}

export function setupStateText(state: SetupState, nowMs: number): string {
  switch (state.state) {
    case "notRun": return "Setup not run"
    case "running": return "Running"
    case "ok": return `Setup ran ${relativeAge(state.endedAtMs, nowMs)}`
    case "failed": return `Setup failed ${relativeAge(state.endedAtMs, nowMs)}`
    case "cancelled": return `Setup cancelled ${relativeAge(state.endedAtMs, nowMs)}`
  }
}

function secretPresenceText(secret: ProjectEnvironmentSecret): string {
  switch (secret.presence) {
    case "present": return "In Vault"
    case "missing": return "Missing here"
    case "locked": return "Vault locked"
  }
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function filePresenceText(file: ProjectEnvironmentFile): string {
  if (file.presence === "missing") return "Missing here"
  return file.sizeBytes === null ? "Present" : `Present · ${formatSize(file.sizeBytes)}`
}

function clip(text: string, width: number): string {
  const flat = text.replace(/\t/gu, "  ")
  return flat.length <= width ? flat : `${flat.slice(0, Math.max(0, width - 1))}…`
}

function twoColumns(left: string, right: string, width: number): string {
  if (!right) return clip(left, width)
  const room = width - right.length - 2
  if (room < 1) return clip(`${left} ${right}`, width)
  return `${clip(left, room).padEnd(room)}  ${right}`
}

function row(text: string, tone: EnvironmentRowTone = "text", focused = false, bold = false): EnvironmentDialogRow {
  return { text, tone, focused, bold }
}

function textFieldRows(
  field: EnvironmentTextField,
  value: string,
  model: EnvironmentDialogModel,
  width: number,
): EnvironmentDialogRow[] {
  const focused = model.focus.kind === field
  const editing = model.input?.kind === "text" && model.input.field === field ? model.input : null
  if (editing) {
    const before = editing.text.slice(0, editing.cursor)
    const lines = `${before}${CURSOR}${editing.text.slice(editing.cursor)}`.split("\n")
    const cursorLine = before.split("\n").length - 1
    const start = Math.max(0, Math.min(cursorLine - FIELD_LINES + 1, lines.length - FIELD_LINES))
    return lines.slice(start, start + FIELD_LINES).map((line) => row(`  ${clip(line, width - 2)}`, "text", true))
  }
  if (!value) return [row("  Empty", "muted", focused)]
  const lines = value.replace(/\n$/u, "").split("\n")
  const shown = lines.slice(0, FIELD_LINES).map((line) => row(`  ${clip(line, width - 2)}`, "text", focused))
  if (lines.length > FIELD_LINES) shown.push(row("  …", "muted", focused))
  return shown
}

function outputRows(operation: EnvironmentOperationView | null, environment: ProjectEnvironmentView, width: number): EnvironmentDialogRow[] {
  const rows: EnvironmentDialogRow[] = []
  if (operation) {
    const lines = operation.outputTail.replace(/\n+$/u, "").split("\n").filter((line) => line.length > 0)
    for (const line of lines.slice(-OUTPUT_LINES)) rows.push(row(`  ${clip(line, width - 2)}`, "muted"))
    if (operation.state === "failed" && operation.reason) rows.push(row(`  ${clip(operation.reason, width - 2)}`, "error"))
    return rows
  }
  const state = environment.setupState
  if (state.state === "failed" && state.reason) rows.push(row(`  ${clip(state.reason, width - 2)}`, "error"))
  return rows
}

function hint(model: EnvironmentDialogModel): string {
  const input = model.input
  if (input?.kind === "text") return "Esc done"
  if (input) return "Enter confirm • Esc cancel"
  const environment = model.environment
  if (!environment) return "Esc close"
  const imports = model.sources.length > 0 && environment.exists
  const focus = model.focus
  const importable = imports && ((focus.kind === "secret" && environment.secrets[focus.index]?.presence === "missing") || (focus.kind === "file" && environment.files[focus.index]?.presence === "missing"))
  const parts = ["↑/↓", "Enter edit", ...(importable ? ["I import here"] : []), ...(imports && model.sources.length > 1 ? ["M machine"] : []), ...(environment.exists ? ["C check"] : []), "X remove", "Esc close"]
  return parts.join(" • ")
}

function windowRows(rows: EnvironmentDialogRow[], pageSize: number): EnvironmentDialogRow[] {
  if (rows.length <= pageSize) return rows
  const focusedAt = rows.findIndex((candidate) => candidate.focused)
  const anchor = focusedAt === -1 ? 0 : focusedAt
  const start = Math.max(0, Math.min(anchor - Math.floor(pageSize / 2), rows.length - pageSize))
  return rows.slice(start, start + pageSize)
}

/** Draft with agent (no Environment yet) or Check if up to date with agent, and what the last run of it says. */
function agentRows(
  model: EnvironmentDialogModel,
  environment: ProjectEnvironmentView,
  width: number,
  focused: (kind: EnvironmentFocus["kind"], index?: number) => boolean,
): EnvironmentDialogRow[] {
  const draft = model.draft
  const running = draft?.state === "running"
  const rows: EnvironmentDialogRow[] = []
  if (!environment.exists && !running) rows.push(row(`  No Environment for this Project yet.`, "muted"))
  const label = environment.exists ? (running ? "Checking with agent" : "Check if up to date with agent") : (running ? "Drafting with agent" : "Draft with agent")
  rows.push(row(twoColumns(`  ${label}`, running ? "Stop" : "", width), "text", focused(environment.exists ? "check" : "draft")))
  if (draft?.state === "failed" && draft.reason) rows.push(row(`  ${clip(draft.reason, width - 2)}`, "error"))
  return rows
}

export function environmentDialogView(model: EnvironmentDialogModel, width: number, pageSize: number): EnvironmentDialogView {
  const environment = model.environment
  if (!environment) {
    return {
      hint: "Esc close",
      rows: [row(model.loading ? "Loading Environment…" : "Environment unavailable", "muted")],
      message: model.message,
    }
  }
  const input = model.input
  const running = environment.setupState.state === "running"
  const focused = (kind: EnvironmentFocus["kind"], index?: number) =>
    model.focus.kind === kind && (index === undefined || ("index" in model.focus && model.focus.index === index))
  const rows: EnvironmentDialogRow[] = []
  const chosen = model.sources[model.sourceIndex]?.label ?? null
  const updating = updatingText(model.transfer, chosen)

  const machine = `${environment.machineLabel} · ${setupStateText(environment.setupState, model.nowMs)}`
  const action = running ? "Stop" : "Run setup"
  // A narrow panel puts the action under the machine line rather than clipping the setup state.
  if (machine.length + 2 + action.length <= width) {
    rows.push(row(twoColumns(machine, action, width), "text", focused("run"), true))
  } else {
    rows.push(row(clip(machine, width), "text", focused("run"), true), row(action, "text", focused("run"), true))
  }

  // One line for Update: it names where the entries would come from, and while it runs it shows the progress and Stop.
  if (model.sources.length > 0 && environment.exists) {
    const failed = model.transfer?.kind === "import" && model.transfer.state === "failed" && model.transfer.entries.length === 0 ? model.transfer.reason : null
    const label = updating ?? failed ?? `Update from ${chosen ?? model.sources[0]!.label}`
    rows.push(row(twoColumns(label, updating ? "Stop" : "Update", width), failed ? "warning" : "text", focused("update"), true))
  }

  const editing = input?.kind === "text" ? input : null
  const scriptDirty = (editing?.field === "script" ? editing.text : model.script) !== environment.setup.script
  const variablesDirty = (editing?.field === "variables" ? editing.text : model.variables) !== environment.variables
  rows.push(row(twoColumns("Setup", scriptDirty ? "Unsaved" : "", width), "text", false, true))
  rows.push(...textFieldRows("script", model.script, model, width))
  rows.push(...outputRows(model.operation, environment, width))
  rows.push(...agentRows(model, environment, width, focused))

  rows.push(row(twoColumns("Variables", variablesDirty ? "Unsaved" : "", width), "text", false, true))
  rows.push(...textFieldRows("variables", model.variables, model, width))

  rows.push(row("Secrets", "text", false, true))
  environment.secrets.forEach((secret, index) => {
    const entering = input?.kind === "secretValue" && input.name === secret.name
    const imported = importRowText(model.transfer, { kind: "secret", name: secret.name }, chosen)
    const status = entering ? `Value ${MASK.repeat(Math.min(input.length, 24))}${CURSOR}` : imported?.text ?? secretPresenceText(secret)
    const problem = !entering && (imported ? imported.problem : secret.presence === "missing")
    rows.push(row(`  ${twoColumns(secret.name, status, width - 2)}`, problem ? "warning" : "text", focused("secret", index)))
  })
  const addingSecret = input?.kind === "line" && input.target === "secretName"
  rows.push(row(addingSecret ? `  Name ${input.text}${CURSOR}` : "  + Add secret", addingSecret ? "text" : "muted", focused("addSecret")))

  rows.push(row("Files outside Git", "text", false, true))
  environment.files.forEach((file, index) => {
    const imported = importRowText(model.transfer, { kind: "file", name: file.path }, chosen)
    rows.push(row(`  ${twoColumns(file.path, imported?.text ?? filePresenceText(file), width - 2)}`, (imported ? imported.problem : file.presence === "missing") ? "warning" : "text", focused("file", index)))
  })
  const addingFile = input?.kind === "line" && input.target === "filePath"
  rows.push(row(addingFile ? `  Path ${input.text}${CURSOR}` : "  + Add file", addingFile ? "text" : "muted", focused("addFile")))

  for (const [title, kind, entries] of [["MCPs", "mcp", environment.mcps], ["Skills", "skill", environment.skills]] as const) {
    if (entries.length === 0) continue
    rows.push(row(title, "text", false, true))
    entries.forEach((entry, index) => {
      rows.push(row(`  ${twoColumns(entry.name, entry.presence === "present" ? "In project files" : "Missing here", width - 2)}`, entry.presence === "present" ? "text" : "warning", focused(kind, index)))
    })
  }

  if (environment.proposals.length > 0) {
    rows.push(row("Proposed", "text", false, true))
    environment.proposals.forEach((proposal, index) => {
      const sign = proposal.change === "add" ? "+" : "−"
      rows.push(row(`  ${twoColumns(`${sign} ${proposal.text.split("\n")[0]}`, `Proposed · ${proposal.evidence.path}`, width - 2)}`, "text", focused("proposal", index)))
    })
  }

  // The failure message sits below the rows with the panel gap above it.
  return { hint: hint(model), rows: windowRows(rows, Math.max(1, pageSize - (model.message ? 2 : 0))), message: model.message }
}
