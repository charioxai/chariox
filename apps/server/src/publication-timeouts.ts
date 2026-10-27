import type { WorkflowPublicationConfig } from "./publication-types.js"

export const DEFAULT_PUBLICATION_WAIT_TIMEOUT_MS = 300_000

export function publicationWaitTimeoutMs(publication: WorkflowPublicationConfig) {
  return publication.sync_timeout_ms ?? DEFAULT_PUBLICATION_WAIT_TIMEOUT_MS
}
