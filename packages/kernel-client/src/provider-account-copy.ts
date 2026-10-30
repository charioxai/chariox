/** Metadata for an explicit copy, never a selectable local account. */
export type ProviderAccountCopyOffer = {
  source_kernel_id: string
  source_machine_id: string
  source_label: string
  source_key_thumbprint: string
  target_kernel_id: string
  provider: string
  account_profile: string
  label: string
}

export type ProviderAccountCopyReceipt = {
  operation_id: string
  provider: string
  source_profile_id: string
  target_kernel_id: string
  account_profile: string
}

export const providerAccountCopyMinimumProtocolVersion = 710
export function listProviderAccountCopyOffersRequest() {
  return { ListProviderAccountCopyOffers: {} }
}
export const providerAccountCopyVaultPromptMinimumProtocolVersion = 711
/**
 * `scope` is where the kernel shows its Vault unlock or create prompt, when the copy needs one. A kernel
 * below 711, or one whose protocol is not known, rejects those fields, so it gets the 710 request without them.
 */
export function copyProviderAccountFromKernelRequest(offer: ProviderAccountCopyOffer, operationId: string, scope: { sessionId?: string; agentId?: string } = {}, kernelProtocol?: number, replace = false) {
  const promptScope = kernelProtocol !== undefined && kernelProtocol >= providerAccountCopyVaultPromptMinimumProtocolVersion ? scope : {}
  return { CopyProviderAccountFromKernel: {
    ...(promptScope.sessionId ? { session_id: promptScope.sessionId } : {}), ...(promptScope.agentId ? { agent_id: promptScope.agentId } : {}),
    provider: offer.provider, account_profile: offer.account_profile,
    source_kernel_id: offer.source_kernel_id, source_key_thumbprint: offer.source_key_thumbprint,
    operation_id: operationId, replace,
  } }
}
