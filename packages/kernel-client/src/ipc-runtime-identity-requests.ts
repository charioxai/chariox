/** Reports kernel-observed identity to its enrollment authority; the authority owns admission policy. */
export type KernelRuntimeIdentityObservationAcknowledgement = {
  readonly environmentId: string
  readonly generation: number
  readonly observedAt: string
}

export function observeKernelRuntimeIdentityRequest(input: {
  readonly environmentId: string
  readonly expectedGeneration: number
}) {
  return { ObserveKernelRuntimeIdentity: input } as const
}
