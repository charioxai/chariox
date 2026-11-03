// Environment v2 wire (local daemon protocol 972). Values of Variables travel in the record;
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

export type ProjectEnvironmentView = {
  readonly projectId: string
  readonly exists: boolean
  readonly revision: number
  readonly setup: { readonly script: string }
  readonly variables: string
  readonly secrets: readonly ProjectEnvironmentSecret[]
  readonly files: readonly ProjectEnvironmentFile[]
  readonly changedAtMs: number
  readonly machineLabel: string
  readonly checkoutPath: string
  readonly setupState: SetupState
}

export type EnvironmentOperationState = "running" | "succeeded" | "failed" | "cancelled"

export type EnvironmentOperationView = {
  readonly operationId: string
  readonly projectId: string
  readonly checkoutPath: string
  readonly kind: "setup"
  readonly state: EnvironmentOperationState
  readonly startedAtMs: number
  readonly endedAtMs: number | null
  readonly exitCode: number | null
  readonly reason: string | null
  readonly outputTail: string
}
