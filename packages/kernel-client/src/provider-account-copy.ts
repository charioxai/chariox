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
export function copyProviderAccountFromKernelRequest(offer: ProviderAccountCopyOffer, operationId: string, replace = false) {
  return { CopyProviderAccountFromKernel: {
    provider: offer.provider, account_profile: offer.account_profile,
    source_kernel_id: offer.source_kernel_id, source_key_thumbprint: offer.source_key_thumbprint,
    operation_id: operationId, replace,
  } }
}
