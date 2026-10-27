import {
  attachToSessionRequest,
  pumpTerminalOutputRequest,
} from "@chariox/kernel-client/ipc-requests"

import type { WorkflowPublicationConfig } from "./publication-types.js"

type KernelPumpClient = {
  send: (request: Record<string, unknown>) => Promise<unknown>
}

const attachedRuntimeAttachments = new Map<string, string>()

export async function pumpPublicationRuntime(
  client: KernelPumpClient,
  publication: WorkflowPublicationConfig,
) {
  const clientId = publicationRuntimeClientId(publication)
  let attachmentId = await ensurePublicationRuntimeAttached(client, publication, clientId)
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      await client.send(pumpTerminalOutputRequest(publication.session_id, attachmentId))
      return
    } catch (error) {
      if (!isAttachmentProjectionRace(error) || attempt === 4) throw error
      attachedRuntimeAttachments.delete(publicationRuntimeAttachmentKey(publication, clientId))
      attachmentId = await ensurePublicationRuntimeAttached(client, publication, clientId)
      await sleep(100)
    }
  }
}

export async function ensurePublicationRuntimeAttached(
  client: KernelPumpClient,
  publication: WorkflowPublicationConfig,
  clientId = publicationRuntimeClientId(publication),
) {
  const attachedKey = publicationRuntimeAttachmentKey(publication, clientId)
  const existingAttachmentId = attachedRuntimeAttachments.get(attachedKey)
  if (existingAttachmentId) return existingAttachmentId
  const response = await client.send(attachToSessionRequest(publication.session_id, clientId))
  const attachmentId = sessionAttachedId(response) ?? clientId
  attachedRuntimeAttachments.set(attachedKey, attachmentId)
  return attachmentId
}

function publicationRuntimeAttachmentKey(publication: WorkflowPublicationConfig, clientId: string) {
  return `${publication.kernel_endpoint ?? ""}:${publication.session_id}:${clientId}`
}

function publicationRuntimeClientId(publication: WorkflowPublicationConfig) {
  const safePublicationId = publication.publication_id.replace(/[^A-Za-z0-9_.-]/g, "_")
  const safeSessionId = publication.session_id.replace(/[^A-Za-z0-9_.-]/g, "_")
  return `chariox-publication-gateway-${process.pid}-${safePublicationId}-${safeSessionId}`
}

function sessionAttachedId(response: unknown) {
  return (response as { SessionAttached?: { attachment?: { id?: string } } } | undefined)
    ?.SessionAttached
    ?.attachment
    ?.id
}

function isAttachmentProjectionRace(error: unknown) {
  const message = error instanceof Error ? error.message : String(error)
  return /attachment `[^`]+` does not belong to session/.test(message)
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
