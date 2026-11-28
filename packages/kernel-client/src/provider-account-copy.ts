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

export const providerAccountCopyMinimumProtocolVersion = 1017
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

export interface ProviderAccountCopyWait {
  readonly operation_id: string
  readonly source_kernel_id: string
  readonly target_kernel_id: string
  readonly target_label: string
  readonly expires_at_ms: number
}

export interface ProviderAccountCopyUnlockRequest {
  readonly interaction_id: string
  readonly expires_at_ms: number
}

export type ProviderAccountCopyResponse =
  | { readonly ProviderAccountCopied: { readonly receipt: ProviderAccountCopyReceipt } }
  | { readonly ProviderAccountCopyWaiting: { readonly wait: ProviderAccountCopyWait } }

/** Watch provider_account_copies on the source kernel to follow a waiting operation. */
export type ProviderAccountCopyProgress =
  | { readonly status: "waiting_for_unlock"; readonly wait: ProviderAccountCopyWait }
  | { readonly status: "installed"; readonly receipt: ProviderAccountCopyReceipt }
  | { readonly status: "failed"; readonly operation_id: string; readonly code: string }
