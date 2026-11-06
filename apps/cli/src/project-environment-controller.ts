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
import type { EnvironmentEntryRef, EnvironmentExtension, EnvironmentOperationView, ProjectEnvironmentView, SetupState } from "@chariox/kernel-client/kernel-types"
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
/** Environment state is kept for this many kernels, the most recently opened ones; one person reaches few kernels from one terminal. */
const RETAINED_KERNELS = 8

type EnvironmentKey = EditKey & { readonly eventType?: string | undefined }

// The secret value lives only in this input state, until it is sent or dropped.
type Input =
  | { readonly kind: "text"; readonly field: EnvironmentTextField; readonly buffer: TextBuffer }
  | { readonly kind: "line"; readonly target: "secretName" | "filePath"; readonly buffer: TextBuffer }
  | { readonly kind: "secretValue"; readonly name: string; readonly buffer: TextBuffer }

type EnvironmentLists = { secretNames: string[]; filePaths: string[]; mcps: EnvironmentExtension[]; skills: EnvironmentExtension[] }
/** A change to the lists, applied to the latest saved ones when it is sent. */
type ListEdit = (lists: EnvironmentLists) => Partial<EnvironmentLists>
/** A completed list edit stays a draft until a save carrying it is acknowledged; `settled` ends once a request carrying it has ended, whatever its outcome. */
type ListDraft = { edit: ListEdit; settled: () => void }

const listsOf = (environment: ProjectEnvironmentView): EnvironmentLists => ({
  secretNames: environment.secrets.map((secret) => secret.name),
  filePaths: environment.files.map((file) => file.path),
  mcps: environment.mcps.map(({ name, source }) => ({ name, source })),
  skills: environment.skills.map(({ name, source }) => ({ name, source })),
})

/** One connection to a kernel; returning to a kernel opens a new one. */
export type KernelClient = { send: (request: unknown) => Promise<unknown> }

export type ProjectEnvironmentControllerDeps = {
  /** The connection the terminal is using now; it changes when the user switches kernels. */
  client: () => KernelClient
  /** The identity of the kernel behind a connection, the same for every connection to that kernel. */
  kernelId: (client: KernelClient) => Promise<string>
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
  let loading = false
  // Edits, saves and operations belong to the Project on one kernel, even while its dialog is closed, and survive leaving that kernel and coming back through a new connection. Each request goes through the client of its editor's kernel when it is sent, so another kernel can neither show nor reach them.
  type Editor = {
    /** The latest connection to the editor's kernel. */
    client: KernelClient
    projectId: string
    environment: ProjectEnvironmentView | null
    typed: { script?: string; variables?: string }
    busy: boolean
    transfer: EnvironmentOperationView | null
    draft: EnvironmentOperationView | null
    // A save was asked for while a request was pending; it runs when that request ends, unless it conflicted.
    saveRequested: boolean
    // Completed list edits not yet acknowledged by the kernel. Every save carries all of them; a conflict or failure keeps them for the next explicit save.
    listDrafts: ListDraft[]
  }
  /** Editors per kernel identity, most recently opened kernel last; only the last `RETAINED_KERNELS` are kept. */
  const editors = new Map<string, Map<string, Editor>>()
  const kernelIds = new WeakMap<KernelClient, Promise<string>>()
  const kernelIdOf = (client: KernelClient): Promise<string> => {
    const known = kernelIds.get(client)
    if (known) return known
    const asked = deps.kernelId(client)
    kernelIds.set(client, asked)
    asked.catch(() => kernelIds.delete(client))
    return asked
  }
  const newEditor = (client: KernelClient, project: string): Editor => ({ client, projectId: project, environment: null, typed: {}, busy: false, saveRequested: false, listDrafts: [], transfer: null, draft: null })
  let editor: Editor = newEditor({ send: async () => { throw new Error("No Environment open") } }, "")
  let focus: EnvironmentFocus = { kind: "run" }
  let input: Input | null = null
  let operation: EnvironmentOperationView | null = null
  let sources: readonly EnvironmentSourceKernel[] = []
  let sourceIndex = 0
  let message: string | null = null
  const pollingOperations = new Map<string, object>()

  const script = (target = editor) => target.typed.script ?? target.environment?.setup.script ?? ""
  const variables = (target = editor) => target.typed.variables ?? target.environment?.variables ?? ""

  const inputView = (): EnvironmentInputView | null => {
    if (!input) return null
    switch (input.kind) {
      case "text": return { kind: "text", field: input.field, text: input.buffer.text, cursor: input.buffer.cursor }
      case "line": return { kind: "line", target: input.target, text: input.buffer.text }
      case "secretValue": return { kind: "secretValue", name: input.name, length: input.buffer.text.length }
    }
  }

  const model = (): EnvironmentDialogModel => ({
    loading, environment: editor.environment, sources, sourceIndex, transfer: editor.transfer, draft: editor.draft, script: script(), variables: variables(), focus, input: inputView(), operation, message, nowMs: deps.now(),
  })

  const focusables = () => environmentFocusables(editor.environment, sources.length > 0 || editor.transfer?.state === "running")

  // Each kind of operation keeps its own slot, so a Draft's answer never replaces the Setup output.
  const setOperation = (next: EnvironmentOperationView, target = editor) => {
    if (next.kind === "import") target.transfer = next
    else if (next.kind === "draft" || next.kind === "check") target.draft = next
    else if (target === editor && open) operation = next
  }

  const setEnvironment = (next: ProjectEnvironmentView) => {
    editor.environment = next
    const keys = focusables().map(focusKey)
    if (!keys.includes(focusKey(focus))) focus = focusables().at(-1) ?? { kind: "run" }
  }

  /** Ordinary requests apply to the current dialog; operation replies also survive closing it. */
  const guarded = async <T>(work: (client: KernelClient) => Promise<T>, apply: (result: T, target: Editor) => void, retainOperation = false): Promise<void> => {
    const mine = generation
    const target = editor
    target.busy = true
    try {
      const result = await work(target.client)
      if (mine !== generation && !retainOperation) return
      if (mine === generation) message = null
      apply(result, target)
    } catch (error) {
      if (mine !== generation) return
      message = failureText(error)
    } finally {
      target.busy = false
      if (target === editor && open) deps.render()
      if (target.saveRequested) { target.saveRequested = false; void saveUnsaved(target) }
    }
  }

  const refresh = async () => {
    const mine = generation
    const target = editor
    try {
      const next = projectEnvironmentFromResponse(await target.client.send(getProjectEnvironmentRequest(target.projectId)))
      if (mine !== generation) return
      if (next.projectId !== target.projectId) throw new Error("Environment unavailable for this Project")
      setEnvironment(next)
      if (next.setupState.state === "running") poll(target, next.setupState.operationId)
      else await showLastRun(target, next.setupState, mine)
    } catch (error) {
      if (mine !== generation) return
      message = failureText(error)
    }
    deps.render()
  }

  // A reopened dialog shows the output of the last finished run; if it cannot be read the state row still tells the result.
  const showLastRun = async (target: Editor, state: SetupState, mine: number) => {
    if (state.state === "notRun" || state.state === "running" || operation?.operationId === state.operationId) return
    try {
      const last = environmentOperationFromResponse(await target.client.send(getEnvironmentOperationRequest(state.operationId)))
      if (mine === generation) operation = last
    } catch {
      // The output is a courtesy.
    }
  }

  // Follows the running operation: the kernel holds each read until the operation has changed, so output and the end show at once.
  const poll = (target: Editor, operationId: string, known?: number) => {
    if (pollingOperations.has(operationId)) return
    const token = {}
    pollingOperations.set(operationId, token)
    const mine = generation
    const current = () => mine === generation && pollingOperations.get(operationId) === token
    const follow = async (seen: number | undefined) => {
      if (!current()) return
      try {
        const next = environmentOperationFromResponse(await target.client.send(getEnvironmentOperationRequest(operationId, seen)))
        if (!current()) return
        setOperation(next, target)
        message = null
        deps.render()
        if (next.state !== "running") {
          pollingOperations.delete(operationId)
          await refresh()
          return
        }
        await follow(next.updates)
        return
      } catch (error) {
        if (!current()) return
        message = failureText(error)
      }
      deps.render()
      // A dropped connection ends the held read; try again after a pause.
      deps.schedule(() => { void follow(undefined) }, OPERATION_RETRY_MS)
    }
    void follow(known)
  }

  /** Keeps `edit` as a draft and saves; resolves once a request carrying it has ended. */
  const save = (edit: ListEdit, target = editor): Promise<void> => new Promise<void>((settled) => {
    target.listDrafts.push({ edit, settled })
    void requestSave(target)
  })

  // While a request is pending the save waits for it to end; the typed text and the drafts are read when it is sent.
  const requestSave = async (target: Editor) => {
    if (target.busy) { target.saveRequested = true; return }
    await send(target)
  }

  const send = async (target: Editor) => {
    const current = target.environment
    const batch = target.listDrafts.slice()
    if (!current) { batch.forEach(({ settled }) => settled()); return }
    const submitted = { script: script(target), variables: variables(target) }
    let acknowledged = false
    target.busy = true
    try {
      const lists = batch.reduce((latest, { edit }) => ({ ...latest, ...edit(latest) }), listsOf(current))
      const outcome = projectEnvironmentSaveOutcomeFromResponse(await target.client.send(saveProjectEnvironmentRequest({
        projectId: target.projectId,
        expectedRevision: current.revision,
        ...submitted,
        ...lists,
      })))
      target.environment = outcome.environment
      if (outcome.kind === "saved") {
        acknowledged = true
        // Only what this request carried is done; edits drafted while it was pending stay.
        target.listDrafts.splice(0, batch.length)
        if (target.typed.script === submitted.script) delete target.typed.script
        if (target.typed.variables === submitted.variables) delete target.typed.variables
      }
      if (target === editor) {
        setEnvironment(outcome.environment)
        message = outcome.kind === "saved" ? null : "Changed elsewhere"
      }
    } catch (error) {
      if (target === editor) message = failureText(error)
    } finally {
      target.busy = false
      batch.forEach(({ settled }) => settled())
      if (target === editor && open) deps.render()
      // After a conflict or failure the drafts wait for an explicit save instead of overwriting the revision again.
      const retry = acknowledged && target.saveRequested
      target.saveRequested = false
      if (retry) void saveUnsaved(target)
    }
  }

  const hasUnsaved = (target = editor) => target.environment !== null && (target.listDrafts.length > 0 || script(target) !== target.environment.setup.script || variables(target) !== target.environment.variables)

  // While a request is pending the text may yet differ from what that request acknowledges, so only an idle editor can tell nothing is unsaved.
  const saveUnsaved = async (target = editor) => {
    if (!target.busy && !hasUnsaved(target)) return
    await requestSave(target)
  }

  const sendSecret = async (name: string, value: string) => {
    await guarded(
      async (client) => projectEnvironmentSavedFromResponse(await client.send(setProjectEnvironmentSecretRequest(editor.projectId, name, value))),
      (next) => setEnvironment(next),
    )
  }

  const toggleSetup = async () => {
    const target = editor
    const mine = generation
    const current = target.environment
    if (!current || editor.busy) return
    if (current.setupState.state === "running") {
      const operationId = current.setupState.operationId
      await guarded(
        async (client) => environmentOperationFromResponse(await client.send(cancelEnvironmentOperationRequest(operationId))),
        (next) => { operation = next; pollingOperations.delete(operationId); void refresh() },
      )
      return
    }
    if (editor.transfer?.state === "running") return
    if (hasUnsaved(target)) {
      await saveUnsaved(target)
      // Closing the initiating dialog abandons this continuation, including a reopen of the same Project.
      if (mine !== generation || hasUnsaved(target)) return
    }
    await guarded(
      async (client) => environmentOperationFromResponse(await client.send(runProjectEnvironmentSetupRequest(target.projectId, deps.newOperationId()))),
      (next) => { operation = next; if (next.state === "running") poll(target, next.operationId, next.updates); void refresh() },
    )
  }

  const source = () => sources[sourceIndex] ?? null

  /** Brings entries this machine lacks from the chosen kernel; no `entries` brings every missing one (Update). */
  const importFromSource = async (entries?: readonly EnvironmentEntryRef[]) => {
    const from = source()
    if (!from || editor.busy || editor.transfer?.state === "running") return
    await guarded(
      async (client) => environmentOperationFromResponse(await client.send(importProjectEnvironmentEntriesRequest({ projectId: editor.projectId, operationId: deps.newOperationId(), sourceKernelId: from.id, ...(entries ? { entries } : {}) }))),
      (next, target) => { setOperation(next, target); if (open && target === editor && next.state === "running") poll(target, next.operationId, next.updates) }, true,
    )
  }

  /** Stop ends the running Import, Draft or Check. */
  const stopOperation = async (running: EnvironmentOperationView | null) => {
    if (!running || running.state !== "running" || editor.busy) return
    await guarded(
      async (client) => environmentOperationFromResponse(await client.send(cancelEnvironmentOperationRequest(running.operationId))),
      (next, target) => { setOperation(next, target); if (open && target === editor) { pollingOperations.delete(running.operationId); void refresh() } }, true,
    )
  }

  const toggleUpdate = async () => { if (editor.transfer?.state === "running") await stopOperation(editor.transfer); else await importFromSource() }

  /** Draft needs an agent; Check compares the project's files itself and adds an agent's judgement when `withAgent`. */
  const runAgent = async (kind: "draft" | "check", withAgent = true) => {
    if (editor.busy) return
    if (editor.draft?.state === "running") { await stopOperation(editor.draft); return }
    await guarded(
      async (client) => environmentOperationFromResponse(await client.send(kind === "draft" ? draftProjectEnvironmentRequest(editor.projectId, deps.newOperationId()) : checkProjectEnvironmentRequest(editor.projectId, deps.newOperationId(), withAgent))),
      (next, target) => { setOperation(next, target); if (open && target === editor && next.state === "running") poll(target, next.operationId, next.updates) }, true,
    )
  }

  const resolveProposal = async (index: number, accept: boolean) => {
    const proposal = editor.environment?.proposals[index]
    if (!proposal || editor.busy) return
    await guarded(
      async (client) => projectEnvironmentSavedFromResponse(await client.send(resolveEnvironmentProposalRequest(editor.projectId, proposal.id, accept))),
      (next) => setEnvironment(next),
    )
  }

  /** Import here: the focused row, when this machine lacks it. */
  const importFocused = () => {
    const current = editor.environment
    if (!current) return
    if (focus.kind === "secret" && current.secrets[focus.index]?.presence === "missing") void importFromSource([{ kind: "secret", name: current.secrets[focus.index]!.name }])
    else if (focus.kind === "file" && current.files[focus.index]?.presence === "missing") void importFromSource([{ kind: "file", name: current.files[focus.index]!.path }])
    else if (focus.kind === "mcp" || focus.kind === "skill") {
      const entry = (focus.kind === "mcp" ? current.mcps : current.skills)[focus.index]
      if (entry && entry.source.kind === "registration" && entry.presence === "missing") void importFromSource([{ kind: focus.kind, name: entry.name }])
    }
  }

  const startText = (field: EnvironmentTextField) => {
    input = { kind: "text", field, buffer: bufferAtEnd(field === "script" ? script() : variables()) }
  }

  const finishText = () => {
    if (input?.kind !== "text") return
    const { field, buffer } = input
    input = null
    editor.typed = field === "script" ? { ...editor.typed, script: buffer.text } : { ...editor.typed, variables: buffer.text }
    void saveUnsaved()
  }

  const submitLine = () => {
    if (input?.kind !== "line" || !editor.environment) return
    const { target, buffer } = input
    const value = buffer.text.trim()
    input = null
    if (!value) return
    if (target === "secretName") {
      const existing = editor.environment.secrets.find((secret) => secret.name === value)
      if (existing) { input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }; return }
      void save(({ secretNames }) => ({ secretNames: secretNames.includes(value) ? secretNames : [...secretNames, value] })).then(() => {
        if (open && editor.environment?.secrets.some((secret) => secret.name === value) && !input) {
          input = { kind: "secretValue", name: value, buffer: bufferAtEnd("") }
          deps.render()
        }
      })
    } else if (!editor.environment.files.some((file) => file.path === value)) {
      void save(({ filePaths }) => ({ filePaths: filePaths.includes(value) ? filePaths : [...filePaths, value] }))
    }
  }

  const submitSecretValue = () => {
    if (input?.kind !== "secretValue") return
    const { name, buffer } = input
    input = null
    if (buffer.text) void sendSecret(name, buffer.text)
  }

  const removeFocused = () => {
    const current = editor.environment
    if (!current) return
    if (focus.kind === "secret") {
      const name = current.secrets[focus.index]?.name
      if (name !== undefined) void save(({ secretNames }) => ({ secretNames: secretNames.filter((other) => other !== name) }))
    } else if (focus.kind === "file") {
      const path = current.files[focus.index]?.path
      if (path !== undefined) void save(({ filePaths }) => ({ filePaths: filePaths.filter((other) => other !== path) }))
    } else if (focus.kind === "mcp") {
      const name = current.mcps[focus.index]?.name
      if (name !== undefined) void save(({ mcps }) => ({ mcps: mcps.filter((other) => other.name !== name) }))
    } else if (focus.kind === "skill") {
      const name = current.skills[focus.index]?.name
      if (name !== undefined) void save(({ skills }) => ({ skills: skills.filter((other) => other.name !== name) }))
    } else if (focus.kind === "proposal") void resolveProposal(focus.index, false)
  }

  const activateFocused = () => {
    const current = editor.environment
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
      pollingOperations.clear()
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
      loading = true
      const client = deps.client()
      // Nothing of the previous kernel shows while this one says who it is.
      editor = newEditor(client, project)
      focus = { kind: "run" }
      input = null
      operation = null
      sources = []
      sourceIndex = 0
      message = null
      pollingOperations.clear()
      deps.render()
      void deps.otherKernels().then((found) => { if (mine === generation) { sources = found; deps.render() } }, () => {})
      try {
        const kernelId = await kernelIdOf(client)
        if (mine !== generation) return
        const onKernel = editors.get(kernelId) ?? new Map<string, Editor>()
        editors.delete(kernelId)
        editors.set(kernelId, onKernel)
        if (editors.size > RETAINED_KERNELS) editors.delete(editors.keys().next().value!)
        const target = onKernel.get(project) ?? editor
        target.client = client
        onKernel.set(project, target)
        editor = target
        const next = projectEnvironmentFromResponse(await client.send(getProjectEnvironmentRequest(project)))
        if (mine !== generation) return
        if (next.projectId !== project) throw new Error("Environment unavailable for this Project")
        if (!target.environment || next.revision >= target.environment.revision) setEnvironment(next)
        for (const running of [target.transfer, target.draft]) {
          if (running?.state === "running") poll(target, running.operationId)
        }
        if (next.setupState.state === "running") poll(target, next.setupState.operationId)
        else await showLastRun(target, next.setupState, mine)
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
      else if (event.name === "c" && !event.ctrl && !event.meta && editor.environment?.exists) void runAgent("check", false)
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
