import {
  cancelEnvironmentOperationRequest,
  environmentOperationFromResponse,
  getEnvironmentOperationRequest,
  getProjectEnvironmentRequest,
  projectEnvironmentFromResponse,
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
  type EnvironmentTextField,
} from "./project-environment-projection.js"
import { applyEditKey, bufferAtEnd, cleanPastedText, insertText, type EditKey, type TextBuffer } from "./text-edit-buffer.js"

const OPERATION_POLL_MS = 1000

type EnvironmentKey = EditKey & { readonly eventType?: string | undefined }

// The secret value lives only in this input state, until it is sent or dropped.
type Input =
  | { readonly kind: "text"; readonly field: EnvironmentTextField; readonly buffer: TextBuffer }
  | { readonly kind: "line"; readonly target: "secretName" | "filePath"; readonly buffer: TextBuffer }
  | { readonly kind: "secretValue"; readonly name: string; readonly buffer: TextBuffer }

type ProjectDraft = {
  readonly projectId: string
  environment: ProjectEnvironmentView | null
  typed: { script?: string; variables?: string }
  busy: boolean
  textSaveQueued: boolean
  rowSaves: Array<() => void>
  message: string | null
}

function projectDraft(projectId: string): ProjectDraft {
  return { projectId, environment: null, typed: {}, busy: false, textSaveQueued: false, rowSaves: [], message: null }
}

export type ProjectEnvironmentControllerDeps = {
  send: (request: unknown) => Promise<unknown>
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
  // Saves belong to the Project and outlive the dialog that submitted them.
  const drafts = new Map<string, ProjectDraft>()
  let draft = projectDraft("")
  let focus: EnvironmentFocus = { kind: "run" }
  let input: Input | null = null
  let operation: EnvironmentOperationView | null = null
  let pollingOperationId: string | null = null
  let submittingLine: Input | null = null

  const script = (state = draft) => state.typed.script ?? state.environment?.setup.script ?? ""
  const variables = (state = draft) => state.typed.variables ?? state.environment?.variables ?? ""

  const inputView = (): EnvironmentInputView | null => {
    if (!input) return null
    switch (input.kind) {
      case "text": return { kind: "text", field: input.field, text: input.buffer.text, cursor: input.buffer.cursor }
      case "line": return { kind: "line", target: input.target, text: input.buffer.text }
      case "secretValue": return { kind: "secretValue", name: input.name, length: input.buffer.text.length }
    }
  }

  const model = (): EnvironmentDialogModel => ({
    loading, environment: draft.environment, script: script(), variables: variables(), focus, input: inputView(), operation, message: draft.message, nowMs: deps.now(),
  })

  const focusables = () => environmentFocusables(draft.environment)

  const setEnvironment = (next: ProjectEnvironmentView, state = draft) => {
    state.environment = next
    if (state !== draft) return
    const keys = focusables().map(focusKey)
    if (!keys.includes(focusKey(focus))) focus = focusables().at(-1) ?? { kind: "run" }
  }

  /** Runs one request for the current dialog; a closed or reopened dialog drops the result. */
  const guarded = async <T>(work: () => Promise<T>, apply: (result: T) => void): Promise<void> => {
    const mine = generation
    const state = draft
    state.busy = true
    try {
      const result = await work()
      if (mine !== generation) return
      state.message = null
      apply(result)
    } catch (error) {
      if (mine !== generation) return
      state.message = failureText(error)
    } finally {
      state.busy = false
      flushSaves(state)
      if (mine !== generation && open && state === draft && !state.busy) void refresh()
      deps.render()
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
      draft.message = failureText(error)
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

  // Polls the running operation about once a second; the output tail is the only live data.
  const poll = (operationId: string) => {
    if (pollingOperationId === operationId) return
    pollingOperationId = operationId
    const mine = generation
    const tick = async () => {
      if (mine !== generation || pollingOperationId !== operationId) return
      try {
        const next = environmentOperationFromResponse(await deps.send(getEnvironmentOperationRequest(operationId)))
        if (mine !== generation || pollingOperationId !== operationId) return
        operation = next
        draft.message = null
        if (next.state !== "running") {
          pollingOperationId = null
          deps.render()
          await refresh()
          return
        }
      } catch (error) {
        if (mine !== generation || pollingOperationId !== operationId) return
        draft.message = failureText(error)
      }
      deps.render()
      deps.schedule(() => { void tick() }, OPERATION_POLL_MS)
    }
    deps.schedule(() => { void tick() }, OPERATION_POLL_MS)
  }

  const save = async (patch: { secretNames?: string[]; filePaths?: string[] }, state = draft) => {
    const current = state.environment
    if (!current || state.busy) return
    const savedScript = script(state)
    const savedVariables = variables(state)
    state.busy = true
    try {
      const outcome = projectEnvironmentSaveOutcomeFromResponse(await deps.send(saveProjectEnvironmentRequest({
        projectId: state.projectId,
        expectedRevision: current.revision,
        script: savedScript,
        variables: savedVariables,
        secretNames: patch.secretNames ?? current.secrets.map((secret) => secret.name),
        filePaths: patch.filePaths ?? current.files.map((file) => file.path),
      })))
      setEnvironment(outcome.environment, state)
      state.message = null
      if (outcome.kind === "saved") {
        if (state.typed.script === savedScript) delete state.typed.script
        if (state.typed.variables === savedVariables) delete state.typed.variables
      } else state.message = "Changed elsewhere"
    } catch (error) {
      state.message = failureText(error)
    } finally {
      state.busy = false
      flushSaves(state)
      deps.render()
    }
  }

  const flushSaves = (state: ProjectDraft) => {
    if (state.busy) return
    const next = state.rowSaves.shift()
    if (next) next()
    else if (state.textSaveQueued) { state.textSaveQueued = false; void saveTypedText(state) }
  }

  // Apply row edits to the acknowledged lists, after any pending save has returned its revision.
  const saveRows = (change: (current: ProjectEnvironmentView) => { secretNames?: string[]; filePaths?: string[] }, onDispatch?: () => void) => {
    const state = draft
    return new Promise<void>((resolve) => {
      state.rowSaves.push(() => {
        onDispatch?.()
        void save(change(state.environment!), state).then(resolve)
      })
      flushSaves(state)
    })
  }

  const hasUnsavedText = (state = draft) => state.environment !== null && (script(state) !== state.environment.setup.script || variables(state) !== state.environment.variables)

  const saveTypedText = async (state = draft) => {
    // Even a return to the loaded value may need saving after the pending request.
    if (state.busy) { state.textSaveQueued = true; return }
    if (hasUnsavedText(state)) await save({}, state)
  }

  const sendSecret = async (name: string, value: string) => {
    await guarded(
      async () => projectEnvironmentSavedFromResponse(await deps.send(setProjectEnvironmentSecretRequest(projectId, name, value))),
      (next) => setEnvironment(next),
    )
  }

  const toggleSetup = async () => {
    const current = draft.environment
    if (!current || draft.busy) return
    if (current.setupState.state === "running") {
      const operationId = current.setupState.operationId
      await guarded(
        async () => environmentOperationFromResponse(await deps.send(cancelEnvironmentOperationRequest(operationId))),
        (next) => { operation = next; pollingOperationId = null; void refresh() },
      )
      return
    }
    if (hasUnsavedText()) {
      await saveTypedText()
      if (hasUnsavedText()) return
    }
    await guarded(
      async () => environmentOperationFromResponse(await deps.send(runProjectEnvironmentSetupRequest(projectId, deps.newOperationId()))),
      (next) => { operation = next; if (next.state === "running") poll(next.operationId); void refresh() },
    )
  }

  const startText = (field: EnvironmentTextField) => {
    input = { kind: "text", field, buffer: bufferAtEnd(field === "script" ? script() : variables()) }
  }

  const finishText = () => {
    if (input?.kind !== "text") return
    const { field, buffer } = input
    input = null
    draft.typed = field === "script" ? { ...draft.typed, script: buffer.text } : { ...draft.typed, variables: buffer.text }
    void saveTypedText()
  }

  const submitLine = () => {
    if (input?.kind !== "line" || !draft.environment || input === submittingLine) return
    const mine = generation
    const submitted = input
    const { target, buffer } = submitted
    const value = buffer.text.trim()
    if (!value) { input = null; return }
    if (target === "secretName" && draft.environment.secrets.some((secret) => secret.name === value)) {
      input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }
      return
    }
    if (target === "filePath" && draft.environment.files.some((file) => file.path === value)) { input = null; return }
    submittingLine = submitted
    const dispatched = () => { if (input === submitted) input = null }
    void saveRows((current) => target === "secretName"
      ? { secretNames: [...new Set([...current.secrets.map((secret) => secret.name), value])] }
      : { filePaths: [...new Set([...current.files.map((file) => file.path), value])] }, dispatched).then(() => {
      if (submittingLine === submitted) submittingLine = null
      if (target === "secretName" && open && mine === generation && draft.environment?.secrets.some((secret) => secret.name === value) && !input) {
        input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }
        deps.render()
      }
    })
  }

  const submitSecretValue = () => {
    if (input?.kind !== "secretValue") return
    const { name, buffer } = input
    input = null
    if (buffer.text) void sendSecret(name, buffer.text)
  }

  const removeFocused = () => {
    const current = draft.environment
    if (!current) return
    if (focus.kind === "secret") {
      const name = current.secrets[focus.index]!.name
      void saveRows((next) => ({ secretNames: next.secrets.filter((secret) => secret.name !== name).map((secret) => secret.name) }))
    } else if (focus.kind === "file") {
      const path = current.files[focus.index]!.path
      void saveRows((next) => ({ filePaths: next.files.filter((file) => file.path !== path).map((file) => file.path) }))
    }
  }

  const activateFocused = () => {
    const current = draft.environment
    if (!current) return
    switch (focus.kind) {
      case "run": void toggleSetup(); return
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
      draft = drafts.get(project) ?? projectDraft(project)
      drafts.set(project, draft)
      if (!hasUnsavedText()) draft.message = null
      loading = !draft.busy
      if (!draft.busy) draft.environment = null
      focus = { kind: "run" }
      input = null
      operation = null
      pollingOperationId = null
      deps.render()
      if (draft.busy) {
        if (draft.environment?.setupState.state === "running") poll(draft.environment.setupState.operationId)
        return
      }
      try {
        const next = projectEnvironmentFromResponse(await deps.send(getProjectEnvironmentRequest(project)))
        if (mine !== generation) return
        if (next.projectId !== project) throw new Error("Environment unavailable for this Project")
        setEnvironment(next)
        if (next.setupState.state === "running") poll(next.setupState.operationId)
        else await showLastRun(next.setupState, mine)
      } catch (error) {
        if (mine !== generation) return
        draft.message = failureText(error)
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
