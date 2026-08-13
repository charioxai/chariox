import type { WorkflowEventBindingStatus } from "./kernel-types.js"

export type EventCatalogSearchOptions = {
  category?: string | null
  verification?: string | null
  cursor?: string | null
  limit?: number
}

export function eventCatalogLandingRequest(limit = 20) {
  return { GetEventGeneratorCatalogLanding: { limit } }
}

export function searchEventGeneratorCatalogRequest(
  query: string,
  options: EventCatalogSearchOptions = {},
) {
  return {
    SearchEventGeneratorCatalog: {
      query,
      category: options.category ?? null,
      verification: options.verification ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 20,
    },
  }
}

export function browseEventGeneratorCategoryRequest(
  category: string,
  options: { cursor?: string | null; limit?: number } = {},
) {
  return {
    BrowseEventGeneratorCategory: {
      category,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 20,
    },
  }
}

export function getEventGeneratorDetailRequest(generatorId: string, version?: string | null) {
  return {
    GetEventGeneratorDetail: {
      generator_id: generatorId,
      version: version ?? null,
    },
  }
}

export function browseEventGeneratorEventsRequest(
  generatorId: string,
  options: { query?: string | null; cursor?: string | null; limit?: number } = {},
) {
  return {
    BrowseEventGeneratorEvents: {
      generator_id: generatorId,
      query: options.query ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 50,
    },
  }
}

export function startEventGeneratorAuthorizationRequest(
  generatorId: string,
  returnUrl?: string | null,
) {
  return {
    StartEventGeneratorAuthorization: {
      generator_id: generatorId,
      return_url: returnUrl ?? null,
    },
  }
}

export function listEventGeneratorResourcesRequest(
  generatorId: string,
  connectionId: string,
  options: { query?: string | null; cursor?: string | null; limit?: number } = {},
) {
  return {
    ListEventGeneratorResources: {
      generator_id: generatorId,
      connection_id: connectionId,
      query: options.query ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 20,
    },
  }
}

export function listEventConnectionsRequest(
  options: { generatorId?: string | null; cursor?: string | null; limit?: number } = {},
) {
  return {
    ListEventConnections: {
      generator_id: options.generatorId ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 20,
    },
  }
}

export function getEventConnectionRequest(connectionId: string) {
  return { GetEventConnection: { connection_id: connectionId } }
}

export function installEventConnectionRequest(generatorId: string, returnUrl?: string | null) {
  return {
    InstallEventConnection: {
      generator_id: generatorId,
      return_url: returnUrl ?? null,
    },
  }
}

export function observeEventConnectionAuthorizationRequest(authorizationId: string) {
  return {
    ObserveEventConnectionAuthorization: { authorization_id: authorizationId },
  }
}

export function refreshEventConnectionRequest(connectionId: string) {
  return { RefreshEventConnection: { connection_id: connectionId } }
}

export function testEventConnectionRequest(connectionId: string, eventType?: string | null) {
  return {
    TestEventConnection: {
      connection_id: connectionId,
      event_type: eventType ?? null,
    },
  }
}

export function reconnectEventConnectionRequest(connectionId: string, returnUrl?: string | null) {
  return {
    ReconnectEventConnection: {
      connection_id: connectionId,
      return_url: returnUrl ?? null,
    },
  }
}

export function listEventConnectionResourcesRequest(
  connectionId: string,
  options: { query?: string | null; cursor?: string | null; limit?: number } = {},
) {
  return {
    ListEventConnectionResources: {
      connection_id: connectionId,
      query: options.query ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? 20,
    },
  }
}

export function listEventConnectionDependenciesRequest(connectionId: string) {
  return { ListEventConnectionDependencies: { connection_id: connectionId } }
}

export function removeEventConnectionRequest(connectionId: string, confirm = false) {
  return { RemoveEventConnection: { connection_id: connectionId, confirm } }
}

export type CreateWorkflowEventBindingInput = {
  generatorId: string
  generatorVersion: string
  manifestDigest: string
  connectionId: string
  connectionScope: string
  eventType: string
  eventTypeVersion: number
  filter?: unknown
  environmentId?: string | null
  queueRef?: string | null
}

export function createWorkflowEventBindingRequest(
  sessionId: string,
  publicationRef: string,
  input: CreateWorkflowEventBindingInput,
) {
  return {
    CreateWorkflowEventBinding: {
      session_id: sessionId,
      publication_ref: publicationRef,
      generator_id: input.generatorId,
      generator_version: input.generatorVersion,
      manifest_digest: input.manifestDigest,
      connection_id: input.connectionId,
      connection_scope: input.connectionScope,
      event_type: input.eventType,
      event_type_version: input.eventTypeVersion,
      filter: input.filter ?? null,
      environment_id: input.environmentId ?? null,
      queue_ref: input.queueRef ?? null,
    },
  }
}

export function listWorkflowEventBindingsRequest(sessionId: string, publicationRef?: string | null) {
  return {
    ListWorkflowEventBindings: {
      session_id: sessionId,
      publication_ref: publicationRef ?? null,
    },
  }
}

export function setWorkflowEventBindingStatusRequest(
  sessionId: string,
  bindingId: string,
  status: WorkflowEventBindingStatus,
) {
  return {
    SetWorkflowEventBindingStatus: {
      session_id: sessionId,
      binding_id: bindingId,
      status,
    },
  }
}

export function transferWorkflowEventBindingRequest(
  sourceSessionId: string,
  bindingId: string,
  targetSessionId: string,
  targetPublicationRef: string,
) {
  return {
    TransferWorkflowEventBinding: {
      source_session_id: sourceSessionId,
      binding_id: bindingId,
      target_session_id: targetSessionId,
      target_publication_ref: targetPublicationRef,
    },
  }
}

export function testWorkflowEventBindingRequest(
  sessionId: string,
  bindingId: string,
  prompt?: string | null,
) {
  return {
    TestWorkflowEventBinding: {
      session_id: sessionId,
      binding_id: bindingId,
      prompt: prompt ?? null,
    },
  }
}

export function getEventDeliveryStatusRequest() {
  return { GetEventDeliveryStatus: {} }
}
