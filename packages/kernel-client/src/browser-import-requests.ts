export const browserImportConsentMinimumProtocolVersion = 315
export const browserImportSourceMinimumProtocolVersion = 315

export type BrowserImportSelection = {
  session_id: string
  attachment_id: string
  environment_id: string
  runtime_generation: number
  tab_id: string
  document_revision: number
  source_store_id: string
  domains: string[]
  partition_sites: string[]
  overwrite: boolean
}

export type BrowserImportConsentResponse = {
  BrowserImportConsent: { request_id: string; status: "prepared" | "approved" | "cancelled" | "source_claimed" | "source_authorized" }
}

// Never spread an input object here: a source adapter may also hold cookie values.
function consentMetadata(selection: BrowserImportSelection): BrowserImportSelection {
  return {
    session_id: selection.session_id,
    attachment_id: selection.attachment_id,
    environment_id: selection.environment_id,
    runtime_generation: selection.runtime_generation,
    tab_id: selection.tab_id,
    document_revision: selection.document_revision,
    source_store_id: selection.source_store_id,
    domains: [...selection.domains],
    partition_sites: [...selection.partition_sites],
    overwrite: selection.overwrite,
  }
}

export function prepareBrowserImportRequest(selection: BrowserImportSelection) {
  return { PrepareBrowserImport: { selection: consentMetadata(selection) } }
}

export function approveBrowserImportRequest(requestId: string, selection: BrowserImportSelection) {
  return { ApproveBrowserImport: { request_id: requestId, selection: consentMetadata(selection) } }
}

export function cancelBrowserImportRequest(sessionId: string, attachmentId: string, requestId: string) {
  return { CancelBrowserImport: { session_id: sessionId, attachment_id: attachmentId, request_id: requestId } }
}

export function claimBrowserImportSourceRequest(requestId: string, selection: BrowserImportSelection) {
  return { ClaimBrowserImportSource: { request_id: requestId, selection: consentMetadata(selection) } }
}

export function authorizeBrowserImportSourceRequest(requestId: string, selection: BrowserImportSelection) {
  return { AuthorizeBrowserImportSource: { request_id: requestId, selection: consentMetadata(selection) } }
}
