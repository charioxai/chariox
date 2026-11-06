import type {
  EnvironmentEntryRef,
  EnvironmentExtension,
  EnvironmentOperationView,
  EnvironmentExtensionSource,
  ProjectEnvironmentExtension,
  ProjectEnvironmentView,
  SetupState,
} from "./kernel-types-project-environment.js"

// Protocol 982 carries the Environment requests including Import, Draft, held operation reads and MCPs and Skills; older kernels answer an unknown-variant error.
export const projectEnvironmentMinimumProtocolVersion = 982

export type SaveProjectEnvironmentInput = {
  readonly projectId: string
  readonly expectedRevision: number
  readonly script: string
  readonly variables: string
  readonly secretNames: readonly string[]
  readonly filePaths: readonly string[]
  /** MCP servers and skills that project files declare, each with the file that does. */
  readonly mcps: readonly EnvironmentExtension[]
  readonly skills: readonly EnvironmentExtension[]
}

export function getProjectEnvironmentRequest(projectId: string, checkoutPath?: string) {
  return { GetProjectEnvironment: { projectId, ...(checkoutPath ? { checkoutPath } : {}) } } as const
}

export function saveProjectEnvironmentRequest(input: SaveProjectEnvironmentInput) {
  return {
    SaveProjectEnvironment: {
      projectId: input.projectId,
      expectedRevision: input.expectedRevision,
      setup: { script: input.script },
      variables: input.variables,
      secrets: input.secretNames.map((name) => ({ name })),
      files: input.filePaths.map((path) => ({ path })),
      mcps: input.mcps.map(({ name, source }) => ({ name, source })),
      skills: input.skills.map(({ name, source }) => ({ name, source })),
    },
  } as const
}

export function setProjectEnvironmentSecretRequest(projectId: string, name: string, value: string) {
  return { SetProjectEnvironmentSecret: { projectId, name, value } } as const
}

export function runProjectEnvironmentSetupRequest(projectId: string, operationId: string, checkoutPath?: string) {
  return { RunProjectEnvironmentSetup: { projectId, operationId, ...(checkoutPath ? { checkoutPath } : {}) } } as const
}

/** Brings entries from another kernel of the same user; no `entries` means everything this kernel lacks. */
export function importProjectEnvironmentEntriesRequest(input: {
  readonly projectId: string
  readonly operationId: string
  readonly sourceKernelId: string
  readonly entries?: readonly EnvironmentEntryRef[]
  readonly checkoutPath?: string
}) {
  return {
    ImportProjectEnvironmentEntries: {
      projectId: input.projectId,
      operationId: input.operationId,
      sourceKernelId: input.sourceKernelId,
      ...(input.entries ? { entries: input.entries } : {}),
      ...(input.checkoutPath ? { checkoutPath: input.checkoutPath } : {}),
    },
  } as const
}

/** An idle agent of the Project on this kernel reads the Project and fills the empty Environment. */
export function draftProjectEnvironmentRequest(projectId: string, operationId: string) {
  return { DraftProjectEnvironment: { projectId, operationId } } as const
}

/** The kernel compares what the Project's files declare with the saved Environment and proposes changes; with `withAgent` an idle agent of the Project adds its own. */
export function checkProjectEnvironmentRequest(projectId: string, operationId: string, withAgent: boolean) {
  return { CheckProjectEnvironment: { projectId, operationId, withAgent } } as const
}

/** Accept applies the proposal as a Save; dismissing keeps it from being proposed again. */
export function resolveEnvironmentProposalRequest(projectId: string, proposalId: string, accept: boolean) {
  return { ResolveEnvironmentProposal: { projectId, proposalId, accept } } as const
}

/** Runs Setup on another kernel of the same user, in the checkout that kernel holds for the Project. */
export function runRemoteProjectEnvironmentSetupRequest(projectId: string, operationId: string, targetKernelId: string, checkoutPath?: string) {
  return { RunRemoteProjectEnvironmentSetup: { projectId, operationId, targetKernelId, ...(checkoutPath ? { checkoutPath } : {}) } } as const
}

/** Reads, or with `stop` ends, an operation that runs on another kernel. */
export function remoteEnvironmentOperationRequest(operationId: string, targetKernelId: string, stop = false) {
  return { RemoteEnvironmentOperation: { operationId, targetKernelId, ...(stop ? { stop } : {}) } } as const
}

/** With `waitAfterUpdates` the kernel holds the answer until the operation has more updates than that, or has ended. */
export function getEnvironmentOperationRequest(operationId: string, waitAfterUpdates?: number) {
  return { GetEnvironmentOperation: { operationId, ...(waitAfterUpdates === undefined ? {} : { waitAfterUpdates }) } } as const
}

export function cancelEnvironmentOperationRequest(operationId: string) {
  return { CancelEnvironmentOperation: { operationId } } as const
}

export type ProjectEnvironmentSaveOutcome =
  | { readonly kind: "saved"; readonly environment: ProjectEnvironmentView }
  | { readonly kind: "conflict"; readonly environment: ProjectEnvironmentView }

type Json = Record<string, unknown>

function isRecord(value: unknown): value is Json {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function field(source: Json, key: string, what: string): unknown {
  if (!Object.hasOwn(source, key)) throw new Error(`${what} is missing "${key}"`)
  return source[key]
}

function text(source: Json, key: string, what: string): string {
  const value = field(source, key, what)
  if (typeof value !== "string") throw new Error(`${what} "${key}" must be text`)
  return value
}

function count(source: Json, key: string, what: string): number {
  const value = field(source, key, what)
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${what} "${key}" must be a non-negative integer`)
  }
  return value
}

function nullableText(source: Json, key: string, what: string): string | null {
  const value = field(source, key, what)
  if (value !== null && typeof value !== "string") throw new Error(`${what} "${key}" must be text or null`)
  return value
}

function nullableCount(source: Json, key: string, what: string): number | null {
  const value = field(source, key, what)
  if (value === null) return null
  return count(source, key, what)
}

function oneOf<T extends string>(source: Json, key: string, allowed: readonly T[], what: string): T {
  const value = text(source, key, what)
  if (!(allowed as readonly string[]).includes(value)) throw new Error(`${what} "${key}" is not one of ${allowed.join(", ")}`)
  return value as T
}

function list(source: Json, key: string, what: string): Json[] {
  const value = field(source, key, what)
  if (!Array.isArray(value) || !value.every(isRecord)) throw new Error(`${what} "${key}" must be a list of objects`)
  return value
}

function object(source: Json, key: string, what: string): Json {
  const value = field(source, key, what)
  if (!isRecord(value)) throw new Error(`${what} "${key}" must be an object`)
  return value
}

function parseSetupState(value: Json): SetupState {
  const what = "Environment setup state"
  switch (oneOf(value, "state", ["notRun", "ok", "failed", "cancelled", "running"], what)) {
    case "notRun": return { state: "notRun" }
    case "ok": return { state: "ok", operationId: text(value, "operationId", what), endedAtMs: count(value, "endedAtMs", what) }
    case "cancelled": return { state: "cancelled", operationId: text(value, "operationId", what), endedAtMs: count(value, "endedAtMs", what) }
    case "failed": return {
      state: "failed",
      operationId: text(value, "operationId", what),
      endedAtMs: count(value, "endedAtMs", what),
      exitCode: nullableInteger(value, "exitCode", what),
      reason: nullableText(value, "reason", what),
    }
    case "running": return {
      state: "running",
      operationId: text(value, "operationId", what),
      startedAtMs: count(value, "startedAtMs", what),
    }
  }
}

function nullableInteger(source: Json, key: string, what: string): number | null {
  const value = field(source, key, what)
  if (value === null) return null
  if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new Error(`${what} "${key}" must be an integer or null`)
  return value
}

function extensionSource(row: Json): EnvironmentExtensionSource {
  const what = "Environment extension source"
  const source = field(row, "source", "Environment extension")
  if (!isRecord(source)) throw new Error(`${what} must be an object`)
  return oneOf(source, "kind", ["file", "registration"], what) === "file"
    ? { kind: "file", path: text(source, "path", what) }
    : { kind: "registration" }
}

function extensionView(row: Json): ProjectEnvironmentExtension {
  return {
    name: text(row, "name", "Environment extension"),
    source: extensionSource(row),
    presence: oneOf(row, "presence", ["present", "missing"], "Environment extension"),
  }
}

export function parseProjectEnvironmentView(value: unknown): ProjectEnvironmentView {
  const what = "Project Environment"
  if (!isRecord(value)) throw new Error(`${what} must be an object`)
  const exists = field(value, "exists", what)
  if (typeof exists !== "boolean") throw new Error(`${what} "exists" must be true or false`)
  return {
    projectId: text(value, "projectId", what),
    exists,
    revision: count(value, "revision", what),
    setup: { script: text(object(value, "setup", what), "script", what) },
    variables: text(value, "variables", what),
    secrets: list(value, "secrets", what).map((row) => ({
      name: text(row, "name", "Environment secret"),
      presence: oneOf(row, "presence", ["present", "missing", "locked"], "Environment secret"),
    })),
    files: list(value, "files", what).map((row) => ({
      path: text(row, "path", "Environment file"),
      presence: oneOf(row, "presence", ["present", "missing"], "Environment file"),
      sizeBytes: nullableCount(row, "sizeBytes", "Environment file"),
    })),
    mcps: list(value, "mcps", what).map(extensionView),
    skills: list(value, "skills", what).map(extensionView),
    changedAtMs: count(value, "changedAtMs", what),
    machineLabel: text(value, "machineLabel", what),
    checkoutPath: text(value, "checkoutPath", what),
    setupState: parseSetupState(object(value, "setupState", what)),
    proposals: list(value, "proposals", what).map(row => {
      const evidence = object(row, "evidence", "Environment proposal")
      return {
        id: text(row, "id", "Environment proposal"),
        field: oneOf(row, "field", ["setup", "variables", "secret", "file", "mcp", "skill"], "Environment proposal"),
        change: oneOf(row, "change", ["add", "remove"], "Environment proposal"),
        text: text(row, "text", "Environment proposal"),
        evidence: { path: text(evidence, "path", "Environment proposal evidence"), excerpt: text(evidence, "excerpt", "Environment proposal evidence") },
      }
    }),
  }
}

export function parseEnvironmentOperationView(value: unknown): EnvironmentOperationView {
  const what = "Environment operation"
  if (!isRecord(value)) throw new Error(`${what} must be an object`)
  return {
    operationId: text(value, "operationId", what),
    projectId: text(value, "projectId", what),
    checkoutPath: text(value, "checkoutPath", what),
    kind: oneOf(value, "kind", ["setup", "import", "draft", "check"], what),
    state: oneOf(value, "state", ["running", "succeeded", "failed", "cancelled"], what),
    startedAtMs: count(value, "startedAtMs", what),
    endedAtMs: nullableCount(value, "endedAtMs", what),
    exitCode: nullableInteger(value, "exitCode", what),
    reason: nullableText(value, "reason", what),
    updates: count(value, "updates", what),
    outputTail: text(value, "outputTail", what),
    entries: list(value, "entries", what).map((row) => ({
      kind: oneOf(row, "kind", ["secret", "file", "mcp", "skill"], "Environment operation entry"),
      name: text(row, "name", "Environment operation entry"),
      state: oneOf(row, "state", ["pending", "importing", "imported", "failed"], "Environment operation entry"),
      reason: nullableText(row, "reason", "Environment operation entry"),
    })),
    sourceLabel: nullableText(value, "sourceLabel", what),
  }
}

function variant(response: unknown, name: string, inner: string): unknown | undefined {
  if (!isRecord(response) || !Object.hasOwn(response, name)) return undefined
  const body = response[name]
  if (!isRecord(body)) throw new Error(`${name} response must be an object`)
  return field(body, inner, `${name} response`)
}

/** Answer of GetProjectEnvironment. */
export function projectEnvironmentFromResponse(response: unknown): ProjectEnvironmentView {
  const environment = variant(response, "ProjectEnvironment", "environment")
  if (environment === undefined) throw new Error("The kernel did not answer with a Project Environment")
  return parseProjectEnvironmentView(environment)
}

/** Answer of SaveProjectEnvironment: saved, or the current record after a stale expectedRevision. */
export function projectEnvironmentSaveOutcomeFromResponse(response: unknown): ProjectEnvironmentSaveOutcome {
  const saved = variant(response, "ProjectEnvironmentSaved", "environment")
  if (saved !== undefined) return { kind: "saved", environment: parseProjectEnvironmentView(saved) }
  const conflict = variant(response, "ProjectEnvironmentConflict", "environment")
  if (conflict !== undefined) return { kind: "conflict", environment: parseProjectEnvironmentView(conflict) }
  throw new Error("The kernel did not answer the Environment save")
}

/** Answer of SetProjectEnvironmentSecret: the record with the refreshed presence. */
export function projectEnvironmentSavedFromResponse(response: unknown): ProjectEnvironmentView {
  const saved = variant(response, "ProjectEnvironmentSaved", "environment")
  if (saved === undefined) throw new Error("The kernel did not confirm the secret")
  return parseProjectEnvironmentView(saved)
}

/** Answer of Run, Get and Cancel. */
export function environmentOperationFromResponse(response: unknown): EnvironmentOperationView {
  const operation = variant(response, "EnvironmentOperation", "operation")
  if (operation === undefined) throw new Error("The kernel did not answer with an Environment operation")
  return parseEnvironmentOperationView(operation)
}
