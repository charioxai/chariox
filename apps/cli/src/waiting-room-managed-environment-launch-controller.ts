import { LocalIpcError } from "./ipc.js"
import type {
  ManagedContextLaunchTarget,
  ManagedContextTransferStatus,
} from "@chariox/kernel-client/ipc-managed-context-requests"
import type {
  ManagedContextTransferTicket,
  ManagedEnvironmentResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { WaitingRoomLaunchConfig } from "./waiting-room-controller.js"

type ManagedEnvironmentLaunchSelection = NonNullable<WaitingRoomLaunchConfig["managedEnvironment"]>

export type PreparedManagedEnvironmentLaunch = {
  readonly environment: ManagedEnvironmentSummary
  readonly launchTarget: ManagedContextLaunchTarget
  readonly workspacePath: string
  readonly worktreePath: string
  commit(): Promise<void>
  rollback(): Promise<void>
}

export type ManagedEnvironmentKernelConnection = {
  commit(): Promise<void>
  rollback(): Promise<void>
}

export type ManagedEnvironmentLaunchAttempt = {
  assertActive(): void
  environmentChanged(environment: ManagedEnvironmentSummary): void
  progress(message: string): void
}

export type WaitingRoomManagedEnvironmentLaunchControllerDeps = {
  createEnvironment(input: {
    clientRequestId: string
    name: string
    region: string
    computeClass: string
    autoStopPolicy: { minimumRuntimeSeconds: number; idleDelaySeconds: number | null }
    contextPlan: Extract<ManagedEnvironmentLaunchSelection, { kind: "new" }>["contextPlan"]
  }): Promise<ManagedEnvironmentResult>
  getEnvironment(environmentId: string): Promise<ManagedEnvironmentSummary>
  requestLifecycle(input: {
    environmentId: string
    action: "start"
    idempotencyKey: string
  }): Promise<ManagedEnvironmentResult>
  prepareContextTransfer(environmentId: string): Promise<ManagedContextTransferTicket>
  startContextTransfer(ticket: ManagedContextTransferTicket): Promise<ManagedContextTransferStatus>
  getContextTransferStatus(contextId: string): Promise<ManagedContextTransferStatus>
  connectKernel(
    machineId: string,
    kernelId: string,
    isActive: () => boolean,
  ): Promise<ManagedEnvironmentKernelConnection | null>
  getLaunchTarget(contextId: string, planDigest: string): Promise<ManagedContextLaunchTarget>
  createIdempotencyKey(): string
  environmentName(): string
  delay(ms: number): Promise<void>
  nowMs(): number
  timeoutMs?: number
}

export class WaitingRoomManagedEnvironmentLaunchController {
  private pendingCreate: { readonly signature: string; readonly key: string } | null = null
  private readonly lifecycleKeys = new Map<string, string>()

  constructor(private readonly deps: WaitingRoomManagedEnvironmentLaunchControllerDeps) {}

  async prepare(
    selection: ManagedEnvironmentLaunchSelection,
    attempt: ManagedEnvironmentLaunchAttempt,
  ): Promise<PreparedManagedEnvironmentLaunch> {
    attempt.assertActive()
    const initial = selection.kind === "new"
      ? await this.create(selection, attempt)
      : await this.deps.getEnvironment(selection.environmentId)
    attempt.assertActive()
    return await this.waitUntilReady(initial, attempt)
  }

  private async create(
    selection: Extract<ManagedEnvironmentLaunchSelection, { kind: "new" }>,
    attempt: ManagedEnvironmentLaunchAttempt,
  ): Promise<ManagedEnvironmentSummary> {
    const requestWithoutId = {
      name: this.deps.environmentName(),
      region: selection.region,
      computeClass: selection.computeClass,
      autoStopPolicy: selection.autoStopPolicy,
      contextPlan: selection.contextPlan,
    }
    const signature = JSON.stringify(requestWithoutId)
    if (!this.pendingCreate || this.pendingCreate.signature !== signature) {
      this.pendingCreate = { signature, key: this.deps.createIdempotencyKey() }
    }
    attempt.progress("Creating managed machine.")
    const result = await this.deps.createEnvironment({
      clientRequestId: this.pendingCreate.key,
      ...requestWithoutId,
    })
    attempt.assertActive()
    this.pendingCreate = null
    attempt.environmentChanged(result.environment)
    return result.environment
  }

  private async waitUntilReady(
    initial: ManagedEnvironmentSummary,
    attempt: ManagedEnvironmentLaunchAttempt,
  ): Promise<PreparedManagedEnvironmentLaunch> {
    const deadline = this.deps.nowMs() + (this.deps.timeoutMs ?? 15 * 60 * 1_000)
    let current = initial
    let transferStatus: ManagedContextTransferStatus | null = null
    let startRequested = false

    while (true) {
      attempt.assertActive()
      attempt.environmentChanged(current)
      if (environmentReadyForSession(current)) {
        return await this.prepareReadyEnvironment(current, deadline, attempt)
      }
      if (current.desiredState === "deleted"
        || current.observedState === "deleted"
        || current.observedState === "deleting") {
        throw new Error(`Managed machine ${current.name} is being deleted.`)
      }
      if (current.observedState === "failed") {
        throw new Error(
          current.lastErrorMessage
          || current.lastErrorCode
          || `Managed machine ${current.name} failed.`,
        )
      }
      if (current.desiredState === "stopped"
        && current.observedState === "stopped"
        && current.desiredRevision === current.observedRevision
        && !startRequested) {
        attempt.progress(`Starting managed machine ${current.name}.`)
        const result = await this.deps.requestLifecycle({
          environmentId: current.environmentId,
          action: "start",
          idempotencyKey: this.lifecycleKey(current),
        })
        attempt.assertActive()
        current = result.environment
        startRequested = true
        continue
      }
      if (current.observedState === "awaiting_context" && current.contextPlan.source) {
        if (!transferStatus) {
          attempt.progress(`Transferring context to ${current.name}.`)
          const ticket = validateTransferTicket(
            await this.deps.prepareContextTransfer(current.environmentId),
            current,
          )
          attempt.assertActive()
          transferStatus = validateTransferStatus(
            await this.deps.startContextTransfer(ticket),
            current,
          )
          attempt.assertActive()
        } else if (transferStatus.phase !== "completed") {
          transferStatus = validateTransferStatus(
            await this.deps.getContextTransferStatus(current.contextPlan.contextId),
            current,
          )
          attempt.assertActive()
        }
        if (transferStatus.phase === "failed") {
          throw new Error(
            transferStatus.failureMessage
            || transferStatus.failureCode
            || "Managed context transfer failed.",
          )
        }
      }
      if (this.deps.nowMs() >= deadline) {
        throw new Error(`Timed out waiting for managed machine ${current.name}.`)
      }
      attempt.progress(environmentProgress(current, transferStatus))
      await this.deps.delay(1_500)
      attempt.assertActive()
      current = await this.deps.getEnvironment(current.environmentId)
      attempt.assertActive()
    }
  }

  private async prepareReadyEnvironment(
    environment: ManagedEnvironmentSummary,
    deadline: number,
    attempt: ManagedEnvironmentLaunchAttempt,
  ): Promise<PreparedManagedEnvironmentLaunch> {
    const machineId = environment.runtimeMachineId as string
    const kernelId = environment.runtimeKernelId as string
    let connection: ManagedEnvironmentKernelConnection
    while (true) {
      attempt.progress(`Connecting to ${environment.name}.`)
      try {
        const connected = await this.deps.connectKernel(machineId, kernelId, () => {
          try {
            attempt.assertActive()
            return true
          } catch {
            return false
          }
        })
        if (connected) {
          connection = connected
          break
        }
      } catch (error) {
        attempt.assertActive()
        if (!(error instanceof LocalIpcError && error.retryable)) {
          throw error
        }
      }
      attempt.assertActive()
      if (this.deps.nowMs() >= deadline) {
        throw new Error(`Managed machine ${environment.name} is ready, but its kernel is not reachable.`)
      }
      await this.deps.delay(1_500)
      attempt.assertActive()
    }

    try {
      attempt.assertActive()
      while (true) {
        try {
          const launchTarget = validateLaunchTarget(
            await this.deps.getLaunchTarget(
              environment.contextPlan.contextId,
              environment.contextPlan.planDigest,
            ),
            environment,
          )
          attempt.assertActive()
          const workspacePath = primaryWorkspacePath(launchTarget)
          return {
            environment,
            launchTarget,
            workspacePath,
            worktreePath: workspacePath,
            commit: connection.commit,
            rollback: connection.rollback,
          }
        } catch (error) {
          attempt.assertActive()
          if (!(error instanceof LocalIpcError && error.retryable)
            || this.deps.nowMs() >= deadline) {
            throw error
          }
          attempt.progress(`Finalizing transferred workspace on ${environment.name}.`)
          await this.deps.delay(1_500)
          attempt.assertActive()
        }
      }
    } catch (error) {
      await connection.rollback()
      throw error
    }
  }

  private lifecycleKey(environment: ManagedEnvironmentSummary): string {
    const identity = `${environment.environmentId}:start:${environment.desiredRevision}`
    let key = this.lifecycleKeys.get(identity)
    if (!key) {
      key = this.deps.createIdempotencyKey()
      this.lifecycleKeys.set(identity, key)
    }
    return key
  }
}

function environmentReadyForSession(environment: ManagedEnvironmentSummary): boolean {
  return environment.desiredState === "running"
    && environment.observedState === "ready"
    && environment.desiredRevision === environment.observedRevision
    && Boolean(environment.runtimeMachineId)
    && Boolean(environment.runtimeKernelId)
    && Boolean(environment.contextManifestDigest)
}

function validateTransferTicket(
  ticket: ManagedContextTransferTicket,
  environment: ManagedEnvironmentSummary,
): ManagedContextTransferTicket {
  if (ticket.environmentId !== environment.environmentId
    || ticket.contextPlan.contextId !== environment.contextPlan.contextId
    || ticket.contextPlan.planDigest !== environment.contextPlan.planDigest
    || ticket.target.machineId !== environment.runtimeMachineId
    || ticket.target.kernelId !== environment.runtimeKernelId) {
    throw new Error("Cloud returned a context transfer ticket for a different managed environment binding.")
  }
  return ticket
}

function validateTransferStatus(
  status: ManagedContextTransferStatus,
  environment: ManagedEnvironmentSummary,
): ManagedContextTransferStatus {
  if (status.contextId !== environment.contextPlan.contextId
    || status.planDigest !== environment.contextPlan.planDigest) {
    throw new Error("The source kernel returned status for a different managed context transfer.")
  }
  return status
}

function validateLaunchTarget(
  target: ManagedContextLaunchTarget,
  environment: ManagedEnvironmentSummary,
): ManagedContextLaunchTarget {
  if (target.environmentId !== environment.environmentId
    || target.kernelId !== environment.runtimeKernelId
    || target.contextId !== environment.contextPlan.contextId
    || target.planDigest !== environment.contextPlan.planDigest) {
    throw new Error("The managed kernel returned a launch target for a different environment binding.")
  }
  const expectedDevelopment = environment.contextPlan.developmentSetup
  if ((expectedDevelopment.kind === "empty") !== (target.development.kind === "empty")) {
    throw new Error("The managed kernel returned a launch target for a different development plan.")
  }
  if (target.development.kind === "from_source") {
    if (expectedDevelopment.kind !== "source_project"
      || target.development.projectId !== expectedDevelopment.projectId
      || target.development.repositories.length !== expectedDevelopment.repositories.length
      || target.development.repositories.some((repository, index) => (
        repository.role !== expectedDevelopment.repositories[index]?.role
      ))) {
      throw new Error("The managed kernel returned different Project repository roles.")
    }
  }
  return target
}

function primaryWorkspacePath(target: ManagedContextLaunchTarget): string {
  const path = target.development.kind === "empty"
    ? target.development.workspacePath
    : target.development.repositories.find((repository) => repository.role === "primary")?.workspacePath
  if (!path?.trim()) {
    throw new Error("The managed kernel returned no launch workspace.")
  }
  return path
}

function environmentProgress(
  environment: ManagedEnvironmentSummary,
  transferStatus: ManagedContextTransferStatus | null,
): string {
  if (transferStatus && transferStatus.phase !== "completed") {
    return `Transferring context to ${environment.name}: ${transferStatus.phase}.`
  }
  switch (environment.observedState) {
    case "requested": return `Waiting to provision ${environment.name}.`
    case "provisioning": return `Provisioning ${environment.name}.`
    case "bootstrapping": return `Bootstrapping ${environment.name}.`
    case "awaiting_context": return `Waiting for context on ${environment.name}.`
    case "starting": return `Starting ${environment.name}.`
    case "stopping": return `Waiting for ${environment.name} to stop before restart.`
    default: return `Waiting for ${environment.name}: ${environment.observedState}.`
  }
}
