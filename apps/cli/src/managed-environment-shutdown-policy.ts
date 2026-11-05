import type { ManagedEnvironmentContextPlanInput } from "@chariox/kernel-client/ipc-managed-environment-requests"

export type ManagedEnvironmentShutdownPolicy =
  | { readonly kind: "time_limit"; readonly durationSeconds: number }
  | { readonly kind: "last_work_finished"; readonly warningSeconds: number }
  | { readonly kind: "manual" }

export type ManagedEnvironmentCreateInput = {
  readonly clientRequestId: string
  readonly name: string
  readonly region: string
  readonly computeClass: string
  readonly managedRepositoryRoot?: string
  readonly shutdownPolicy: ManagedEnvironmentShutdownPolicy
  readonly contextPlan: ManagedEnvironmentContextPlanInput
}
