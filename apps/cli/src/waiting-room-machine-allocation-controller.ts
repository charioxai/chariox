import type { MachineAllocation, MachineAllocationCapability } from "./machine-allocation-api.js"

export type MachineAllocationAttempt = {
  assertActive(): void
  allocated(machine: MachineAllocation): void
  progress(message: string): void
}
export type MachineAllocationControllerDeps = {
  acquire(capabilityRevision: string, requestId: string): Promise<MachineAllocation>
  get(allocationRef: string): Promise<MachineAllocation>
  requestId(): string
  now(): number
  delay(ms: number): Promise<void>
}

export class WaitingRoomMachineAllocationController {
  private pending: { capabilityRevision: string; requestId: string; allocationRef?: string } | undefined

  constructor(private readonly deps: MachineAllocationControllerDeps) {}

  complete(allocationRef: string): void {
    if (this.pending?.allocationRef === allocationRef) this.pending = undefined
  }

  async acquire(capability: MachineAllocationCapability, attempt: MachineAllocationAttempt): Promise<MachineAllocation & { kernel_target: NonNullable<MachineAllocation["kernel_target"]> }> {
    attempt.assertActive()
    if (!this.pending && (!capability.actions.includes("acquire") || !capability.offer)) throw new Error("The connected service is not offering a new machine.")
    if (!this.pending) this.pending = { capabilityRevision: capability.revision, requestId: this.deps.requestId() }
    const pending = this.pending
    let machine = pending.allocationRef
      ? await this.deps.get(pending.allocationRef)
      : await this.deps.acquire(pending.capabilityRevision, pending.requestId)
    // Retain the allocation even if the person changed selection while acquisition was in flight.
    // Retrying this attempt must observe the same machine rather than acquire another one.
    const allocationRef = machine.allocation_ref
    pending.allocationRef = allocationRef
    const deadline = this.deps.now() + 15 * 60 * 1000
    while (true) {
      attempt.assertActive()
      attempt.allocated(machine)
      if (machine.access_state === "available" && machine.kernel_target) {
        return { ...machine, kernel_target: machine.kernel_target }
      }
      if (["failed", "deleted", "deleting"].includes(machine.state) || machine.access_state === "denied") {
        this.pending = undefined
        throw new Error(machine.access_reason || "The connected service could not prepare this machine.")
      }
      if (this.deps.now() >= deadline) throw new Error("Timed out waiting for the new machine. Retry to check the same allocation.")
      attempt.progress("Preparing the new machine.")
      await this.deps.delay(1500)
      attempt.assertActive()
      machine = await this.deps.get(allocationRef)
      attempt.assertActive()
    }
  }
}
