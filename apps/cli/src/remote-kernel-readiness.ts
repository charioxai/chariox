export type RemoteKernelReadiness = "ready" | "blocked" | "needs-provider" | "unknown"

export type RemoteKernelReadinessInput = {
  accepting_remote_leases?: boolean
  available_providers?: string[]
}

export function remoteKernelReadiness(kernel: RemoteKernelReadinessInput): RemoteKernelReadiness {
  if (kernel.accepting_remote_leases === false) return "blocked"
  if ((kernel.available_providers ?? []).length === 0) {
    return kernel.accepting_remote_leases === undefined ? "unknown" : "needs-provider"
  }
  if (kernel.accepting_remote_leases === undefined) return "unknown"
  return "ready"
}

export function remoteKernelReadinessCounts(kernels: readonly RemoteKernelReadinessInput[]): Record<RemoteKernelReadiness, number> {
  return kernels.reduce<Record<RemoteKernelReadiness, number>>((counts, kernel) => {
    counts[remoteKernelReadiness(kernel)] += 1
    return counts
  }, { ready: 0, blocked: 0, "needs-provider": 0, unknown: 0 })
}
