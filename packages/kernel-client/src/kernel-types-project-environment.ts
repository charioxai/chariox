// Environment v2 wire (local daemon protocol 973). Values of Variables travel in the record;
// the value of a Secret never leaves the Vault, so no shape here can carry one.

export type EnvironmentSecretPresence = "present" | "missing" | "locked"
export type EnvironmentFilePresence = "present" | "missing"

export type ProjectEnvironmentSecret = {
  readonly name: string
  readonly presence: EnvironmentSecretPresence
}

export type ProjectEnvironmentFile = {
  readonly path: string
  readonly presence: EnvironmentFilePresence
  readonly sizeBytes: number | null
}

export type SetupState =
  | { readonly state: "notRun" }
  | { readonly state: "ok"; readonly operationId: string; readonly endedAtMs: number }
  | { readonly state: "failed"; readonly operationId: string; readonly endedAtMs: number; readonly exitCode: number | null; readonly reason: string | null }
  | { readonly state: "cancelled"; readonly operationId: string; readonly endedAtMs: number }
  | { readonly state: "running"; readonly operationId: string; readonly startedAtMs: number }

export type EnvironmentProposalField = "setup" | "variables" | "secret" | "file" | "mcp" | "skill"

/** One change an agent proposed: a line of Setup or Variables, a Secret name or a File path, with the line that shows why. */
export type EnvironmentProposal = {
  readonly id: string
  readonly field: EnvironmentProposalField
  readonly change: "add" | "remove"
  readonly text: string
  readonly evidence: { readonly path: string; readonly excerpt: string }
}

/** An MCP server or skill that a project file declares; `presence` says whether that file still declares it here. */
export type ProjectEnvironmentExtension = { readonly name: string; readonly source: string; readonly presence: "present" | "missing" }
export type EnvironmentExtension = { readonly name: string; readonly source: string }

export type ProjectEnvironmentView = {
  readonly projectId: string
  readonly exists: boolean
  readonly revision: number
  readonly setup: { readonly script: string }
  readonly variables: string
  readonly secrets: readonly ProjectEnvironmentSecret[]
  readonly files: readonly ProjectEnvironmentFile[]
  readonly mcps: readonly ProjectEnvironmentExtension[]
  readonly skills: readonly ProjectEnvironmentExtension[]
  readonly changedAtMs: number
  readonly machineLabel: string
  readonly checkoutPath: string
  readonly setupState: SetupState
  /** Changes an agent proposed that nobody has accepted or dismissed yet. */
  readonly proposals: readonly EnvironmentProposal[]
}

export type EnvironmentOperationState = "running" | "succeeded" | "failed" | "cancelled"
export type EnvironmentOperationKind = "setup" | "import" | "draft" | "check"
export type EnvironmentEntryKind = "secret" | "file"
export type EnvironmentEntryState = "pending" | "importing" | "imported" | "failed"

/** One Secret (by name) or File (by relative path) of an Environment. */
export type EnvironmentEntryRef = {
  readonly kind: EnvironmentEntryKind
  readonly name: string
}

/** One entry of an Import and where it stands; `reason` says why a failed one failed. */
export type EnvironmentOperationEntry = EnvironmentEntryRef & {
  readonly state: EnvironmentEntryState
  readonly reason: string | null
}

export type EnvironmentOperationView = {
  readonly operationId: string
  readonly projectId: string
  readonly checkoutPath: string
  readonly kind: EnvironmentOperationKind
  readonly state: EnvironmentOperationState
  readonly startedAtMs: number
  readonly endedAtMs: number | null
  readonly exitCode: number | null
  readonly reason: string | null
  /** Counts every change of the operation, so a reader can wait for the next one. */
  readonly updates: number
  readonly outputTail: string
  /** Import only. */
  readonly entries: readonly EnvironmentOperationEntry[]
  /** Import only: the display name of the kernel the entries come from. */
  readonly sourceLabel: string | null
}
