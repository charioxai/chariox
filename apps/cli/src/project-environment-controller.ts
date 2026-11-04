import {
  cancelEnvironmentOperationRequest,
  checkProjectEnvironmentRequest,
  draftProjectEnvironmentRequest,
  environmentOperationFromResponse,
  getEnvironmentOperationRequest,
  getProjectEnvironmentRequest,
  importProjectEnvironmentEntriesRequest,
  projectEnvironmentFromResponse,
  resolveEnvironmentProposalRequest,
  projectEnvironmentSaveOutcomeFromResponse,
  projectEnvironmentSavedFromResponse,
  runProjectEnvironmentSetupRequest,
  saveProjectEnvironmentRequest,
  setProjectEnvironmentSecretRequest,
} from "@chariox/kernel-client/ipc-project-environment-requests"
import type { EnvironmentOperationView, ProjectEnvironmentView, SetupState } from "@chariox/kernel-client/kernel-types"
import {
  environmentDialogView,
  environmentFocusables,
  focusKey,
  type EnvironmentDialogModel,
  type EnvironmentDialogView,
  type EnvironmentFocus,
  type EnvironmentInputView,
  type EnvironmentSourceKernel,
  type EnvironmentTextField,
} from "./project-environment-projection.js"
import { applyEditKey, bufferAtEnd, cleanPastedText, insertText, type EditKey, type TextBuffer } from "./text-edit-buffer.js"

const OPERATION_RETRY_MS = 1000

type EnvironmentKey = EditKey & { readonly eventType?: string | undefined }

// The secret value lives only in this input state, until it is sent or dropped.
type Input =
  | { readonly kind: "text"; readonly field: EnvironmentTextField; readonly buffer: TextBuffer }
  | { readonly kind: "line"; readonly target: "secretName" | "filePath"; readonly buffer: TextBuffer }
  | { readonly kind: "secretValue"; readonly name: string; readonly buffer: TextBuffer }

export type ProjectEnvironmentControllerDeps = {
  send: (request: unknown) => Promise<unknown>
  /** The user's other connected kernels, from the same Cloud directory the web lists; none when it cannot be read. */
  otherKernels: () => Promise<readonly EnvironmentSourceKernel[]>
  render: () => void
  schedule: (callback: () => void, delayMs: number) => unknown
  newOperationId: () => string
  now: () => number
  pageSize: () => number
  textWidth: () => number
}

function failureText(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "Request failed"
}

/** The TUI's Environment dialog for the kernel it is connected to. */
export function createProjectEnvironmentController(deps: ProjectEnvironmentControllerDeps) {
  let open = false
  let generation = 0
  let projectId = ""
  let loading = false
  let environment: ProjectEnvironmentView | null = null
  // Text the user typed and has not saved; a field absent here shows the saved record.
  let typed: { script?: string; variables?: string } = {}
  let focus: EnvironmentFocus = { kind: "run" }
  let input: Input | null = null
  let operation: EnvironmentOperationView | null = null
  let transfer: EnvironmentOperationView | null = null
  let draft: EnvironmentOperationView | null = null
  let sources: readonly EnvironmentSourceKernel[] = []
  let sourceIndex = 0
  let message: string | null = null
  let busy = false
  let pollingOperationId: string | null = null

  const script = () => typed.script ?? environment?.setup.script ?? ""
  const variables = () => typed.variables ?? environment?.variables ?? ""

  const inputView = (): EnvironmentInputView | null => {
    if (!input) return null
    switch (input.kind) {
      case "text": return { kind: "text", field: input.field, text: input.buffer.text, cursor: input.buffer.cursor }
      case "line": return { kind: "line", target: input.target, text: input.buffer.text }
      case "secretValue": return { kind: "secretValue", name: input.name, length: input.buffer.text.length }
    }
  }

  const model = (): EnvironmentDialogModel => ({
    loading, environment, sources, sourceIndex, transfer, draft, script: script(), variables: variables(), focus, input: inputView(), operation, message, nowMs: deps.now(),
  })

  const focusables = () => environmentFocusables(environment, sources.length > 0)

  // Each kind of operation keeps its own slot, so a Draft's answer never replaces the Setup output.
  const setOperation = (next: EnvironmentOperationView) => {
    if (next.kind === "import") transfer = next
    else if (next.kind === "draft" || next.kind === "check") draft = next
    else operation = next
  }

  const setEnvironment = (next: ProjectEnvironmentView) => {
    environment = next
    const keys = focusables().map(focusKey)
    if (!keys.includes(focusKey(focus))) focus = focusables().at(-1) ?? { kind: "run" }
  }

  /** Runs one request for the current dialog; a closed or reopened dialog drops the result. */
  const guarded = async <T>(work: () => Promise<T>, apply: (result: T) => void): Promise<void> => {
    const mine = generation
    busy = true
    try {
      const result = await work()
      if (mine !== generation) return
      message = null
      apply(result)
    } catch (error) {
      if (mine !== generation) return
      message = failureText(error)
    } finally {
      if (mine === generation) { busy = false; deps.render() }
    }
  }

  const refresh = async () => {
    const mine = generation
    try {
      const next = projectEnvironmentFromResponse(await deps.send(getProjectEnvironmentRequest(projectId)))
      if (mine !== generation) return
      if (next.projectId !== projectId) throw new Error("Environment unavailable for this Project")
      setEnvironment(next)
      if (next.setupState.state === "running") poll(next.setupState.operationId)
      else await showLastRun(next.setupState, mine)
    } catch (error) {
      if (mine !== generation) return
      message = failureText(error)
    }
    deps.render()
  }

  // A reopened dialog shows the output of the last finished run; if it cannot be read the state row still tells the result.
  const showLastRun = async (state: SetupState, mine: number) => {
    if (state.state === "notRun" || state.state === "running" || operation?.operationId === state.operationId) return
    try {
      const last = environmentOperationFromResponse(await deps.send(getEnvironmentOperationRequest(state.operationId)))
      if (mine === generation) operation = last
    } catch {
      // The output is a courtesy.
    }
  }

  // Follows the running operation: the kernel holds each read until the operation has changed, so output and the end show at once.
  const poll = (operationId: string, known?: number) => {
    if (pollingOperationId === operationId) return
    pollingOperationId = operationId
    const mine = generation
    const follow = async (seen: number | undefined) => {
      if (mine !== generation || pollingOperationId !== operationId) return
      try {
        const next = environmentOperationFromResponse(await deps.send(getEnvironmentOperationRequest(operationId, seen)))
        if (mine !== generation || pollingOperationId !== operationId) return
        setOperation(next)
        message = null
        deps.render()
        if (next.state !== "running") {
          pollingOperationId = null
          await refresh()
          return
        }
        await follow(next.updates)
        return
      } catch (error) {
        if (mine !== generation || pollingOperationId !== operationId) return
        message = failureText(error)
      }
      deps.render()
      // A dropped connection ends the held read; try again after a pause.
      deps.schedule(() => { void follow(undefined) }, OPERATION_RETRY_MS)
    }
    void follow(known)
  }

  const save = async (patch: { secretNames?: string[]; filePaths?: string[]; mcps?: { name: string; source: string }[]; skills?: { name: string; source: string }[] }) => {
    const current = environment
    if (!current || busy) return
    await guarded(
      async () => projectEnvironmentSaveOutcomeFromResponse(await deps.send(saveProjectEnvironmentRequest({
        projectId,
        expectedRevision: current.revision,
        script: script(),
        variables: variables(),
        secretNames: patch.secretNames ?? current.secrets.map((secret) => secret.name),
        filePaths: patch.filePaths ?? current.files.map((file) => file.path),
        mcps: patch.mcps ?? current.mcps.map(({ name, source }) => ({ name, source })),
        skills: patch.skills ?? current.skills.map(({ name, source }) => ({ name, source })),
      }))),
      (outcome) => {
        setEnvironment(outcome.environment)
        if (outcome.kind === "saved") typed = {}
        else message = "Changed elsewhere"
      },
    )
  }

  const hasUnsavedText = () => environment !== null && (script() !== environment.setup.script || variables() !== environment.variables)

  const saveTypedText = async () => {
    if (hasUnsavedText()) await save({})
  }

  const sendSecret = async (name: string, value: string) => {
    await guarded(
      async () => projectEnvironmentSavedFromResponse(await deps.send(setProjectEnvironmentSecretRequest(projectId, name, value))),
      (next) => setEnvironment(next),
    )
  }

  const toggleSetup = async () => {
    const current = environment
    if (!current || busy) return
    if (current.setupState.state === "running") {
      const operationId = current.setupState.operationId
      await guarded(
        async () => environmentOperationFromResponse(await deps.send(cancelEnvironmentOperationRequest(operationId))),
        (next) => { operation = next; pollingOperationId = null; void refresh() },
      )
      return
    }
    if (transfer?.state === "running") return
    if (hasUnsavedText()) {
      await saveTypedText()
      if (hasUnsavedText()) return
    }
    await guarded(
      async () => environmentOperationFromResponse(await deps.send(runProjectEnvironmentSetupRequest(projectId, deps.newOperationId()))),
      (next) => { operation = next; if (next.state === "running") poll(next.operationId, next.updates); void refresh() },
    )
  }

  const source = () => sources[sourceIndex] ?? null

  /** Brings entries this machine lacks from the chosen kernel; no `entries` brings every missing one (Update). */
  const importFromSource = async (entries?: readonly { kind: "secret" | "file"; name: string }[]) => {
    const from = source()
    if (!from || busy || transfer?.state === "running") return
    await guarded(
      async () => environmentOperationFromResponse(await deps.send(importProjectEnvironmentEntriesRequest({ projectId, operationId: deps.newOperationId(), sourceKernelId: from.id, ...(entries ? { entries } : {}) }))),
      (next) => { setOperation(next); if (next.state === "running") poll(next.operationId, next.updates) },
    )
  }

  /** Stop ends the running Import, Draft or Check. */
  const stopOperation = async (running: EnvironmentOperationView | null) => {
    if (!running || running.state !== "running" || busy) return
    await guarded(
      async () => environmentOperationFromResponse(await deps.send(cancelEnvironmentOperationRequest(running.operationId))),
      (next) => { setOperation(next); pollingOperationId = null; void refresh() },
    )
  }

  const toggleUpdate = async () => { if (transfer?.state === "running") await stopOperation(transfer); else await importFromSource() }

  /** Draft needs an agent; Check compares the project's files itself and adds an agent's judgement when `withAgent`. */
  const runAgent = async (kind: "draft" | "check", withAgent = true) => {
    if (busy) return
    if (draft?.state === "running") { await stopOperation(draft); return }
    await guarded(
      async () => environmentOperationFromResponse(await deps.send(kind === "draft" ? draftProjectEnvironmentRequest(projectId, deps.newOperationId()) : checkProjectEnvironmentRequest(projectId, deps.newOperationId(), withAgent))),
      (next) => { setOperation(next); if (next.state === "running") poll(next.operationId, next.updates) },
    )
  }

  const resolveProposal = async (index: number, accept: boolean) => {
    const proposal = environment?.proposals[index]
    if (!proposal || busy) return
    await guarded(
      async () => projectEnvironmentSavedFromResponse(await deps.send(resolveEnvironmentProposalRequest(projectId, proposal.id, accept))),
      (next) => setEnvironment(next),
    )
  }

  /** Import here: the focused row, when this machine lacks it. */
  const importFocused = () => {
    const current = environment
    if (!current) return
    if (focus.kind === "secret" && current.secrets[focus.index]?.presence === "missing") void importFromSource([{ kind: "secret", name: current.secrets[focus.index]!.name }])
    else if (focus.kind === "file" && current.files[focus.index]?.presence === "missing") void importFromSource([{ kind: "file", name: current.files[focus.index]!.path }])
  }

  const startText = (field: EnvironmentTextField) => {
    input = { kind: "text", field, buffer: bufferAtEnd(field === "script" ? script() : variables()) }
  }

  const finishText = () => {
    if (input?.kind !== "text") return
    const { field, buffer } = input
    input = null
    typed = field === "script" ? { ...typed, script: buffer.text } : { ...typed, variables: buffer.text }
    void saveTypedText()
  }

  const submitLine = () => {
    if (input?.kind !== "line" || !environment) return
    const { target, buffer } = input
    const value = buffer.text.trim()
    input = null
    if (!value) return
    if (target === "secretName") {
      const existing = environment.secrets.find((secret) => secret.name === value)
      if (existing) { input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }; return }
      void save({ secretNames: [...environment.secrets.map((secret) => secret.name), value] }).then(() => {
        if (open && environment?.secrets.some((secret) => secret.name === value) && !input) {
          input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }
          deps.render()
        }
      })
    } else if (!environment.files.some((file) => file.path === value)) {
      void save({ filePaths: [...environment.files.map((file) => file.path), value] })
    }
  }

  const submitSecretValue = () => {
    if (input?.kind !== "secretValue") return
    const { name, buffer } = input
    input = null
    if (buffer.text) void sendSecret(name, buffer.text)
  }

  const removeFocused = () => {
    const current = environment
    if (!current) return
    if (focus.kind === "secret") {
      const index = focus.index
      void save({ secretNames: current.secrets.filter((_, at) => at !== index).map((secret) => secret.name) })
    } else if (focus.kind === "file") {
      const index = focus.index
      void save({ filePaths: current.files.filter((_, at) => at !== index).map((file) => file.path) })
    } else if (focus.kind === "mcp") {
      const index = focus.index
      void save({ mcps: current.mcps.filter((_, at) => at !== index).map(({ name, source }) => ({ name, source })) })
    } else if (focus.kind === "skill") {
      const index = focus.index
      void save({ skills: current.skills.filter((_, at) => at !== index).map(({ name, source }) => ({ name, source })) })
    } else if (focus.kind === "proposal") void resolveProposal(focus.index, false)
  }

  const activateFocused = () => {
    const current = environment
    if (!current) return
    switch (focus.kind) {
      case "run": void toggleSetup(); return
      case "update": void toggleUpdate(); return
      case "draft": void runAgent("draft"); return
      case "check": void runAgent("check"); return
      case "proposal": void resolveProposal(focus.index, true); return
      case "mcp": case "skill": return
      case "script": startText("script"); return
      case "variables": startText("variables"); return
      case "secret": input = { kind: "secretValue", name: current.secrets[focus.index]!.name, buffer: bufferAtEnd("") }; return
      case "addSecret": input = { kind: "line", target: "secretName", buffer: bufferAtEnd("") }; return
      case "addFile": input = { kind: "line", target: "filePath", buffer: bufferAtEnd("") }; return
      case "file": return
    }
  }

  const moveFocus = (delta: number) => {
    const all = focusables()
    if (all.length === 0) return
    const at = Math.max(0, all.map(focusKey).indexOf(focusKey(focus)))
    focus = all[Math.max(0, Math.min(all.length - 1, at + delta))]!
  }

  const controller = {
    isOpen: () => open,
    view(): EnvironmentDialogView {
      return environmentDialogView(model(), deps.textWidth(), deps.pageSize())
    },
    close() {
      open = false
      generation++
      input = null
      pollingOperationId = null
      deps.render()
    },
    /** Esc: leaves the active input first, closes the dialog when none is active. */
    dismiss() {
      if (input?.kind === "text") { finishText(); deps.render(); return }
      if (input) { input = null; deps.render(); return }
      controller.close()
    },
    async open(project: string) {
      const mine = ++generation
      open = true
      projectId = project
      loading = true
      environment = null
      typed = {}
      focus = { kind: "run" }
      input = null
      operation = null
      transfer = null
      draft = null
      sources = []
      sourceIndex = 0
      message = null
      busy = false
      pollingOperationId = null
      deps.render()
      void deps.otherKernels().then((found) => { if (mine === generation) { sources = found; deps.render() } }, () => {})
      try {
        const next = projectEnvironmentFromResponse(await deps.send(getProjectEnvironmentRequest(project)))
        if (mine !== generation) return
        if (next.projectId !== project) throw new Error("Environment unavailable for this Project")
        setEnvironment(next)
        if (next.setupState.state === "running") poll(next.setupState.operationId)
        else await showLastRun(next.setupState, mine)
      } catch (error) {
        if (mine !== generation) return
        message = failureText(error)
      }
      loading = false
      deps.render()
    },
    handlePaste(text: string): boolean {
      if (!open || !input) return false
      const multiline = input.kind === "text"
      input = { ...input, buffer: insertText(input.buffer, cleanPastedText(text, multiline)) }
      deps.render()
      return true
    },
    handleKey(event: EnvironmentKey): boolean {
      if (!open) return false
      if (event.eventType === "release") return true
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.name === "escape") { controller.dismiss(); return true }
      if (input) {
        handleInputKey(event)
        deps.render()
        return true
      }
      if (event.name === "up") moveFocus(-1)
      else if (event.name === "down") moveFocus(1)
      else if (event.name === "return" || event.name === "enter") activateFocused()
      else if (["x", "delete", "backspace"].includes(event.name) && !event.ctrl && !event.meta) removeFocused()
      else if (event.name === "i" && !event.ctrl && !event.meta) importFocused()
      else if (event.name === "c" && !event.ctrl && !event.meta && environment?.exists) void runAgent("check", false)
      else if (event.name === "m" && !event.ctrl && !event.meta && sources.length > 1) sourceIndex = (sourceIndex + 1) % sources.length
      deps.render()
      return true
    },
  }

  const handleInputKey = (event: EnvironmentKey) => {
    if (!input) return
    const current = input
    const submits = event.name === "return" || event.name === "enter"
    if (submits && current.kind === "line") { submitLine(); return }
    if (submits && current.kind === "secretValue") { submitSecretValue(); return }
    const next = applyEditKey(current.buffer, event, current.kind === "text")
    if (next) input = { ...current, buffer: next }
  }

  return controller
}
