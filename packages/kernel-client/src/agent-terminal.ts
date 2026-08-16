import { readFile, realpath } from "node:fs/promises"
import { isAbsolute, relative, resolve as resolvePath } from "node:path"

import { getTerminalCommandCatalogRequest, getTerminalOperationRegistryRequest, pumpTerminalOutputRequest } from "./ipc-terminal-runtime-requests.js"
import { attachToSessionRequest, detachFromSessionRequest, getSessionStateRequest } from "./ipc-session-requests.js"
import {
  createDefaultShellContext,
  parseShellCommand,
  type ShellContext,
} from "./shell-core.js"
import { executeShellLine, type ShellScriptOptions } from "./shell-script.js"
import type { TerminalOperationContract, TerminalOperationRegistry } from "./kernel-types-terminal.js"
import type { KernelEvent } from "./kernel-events.js"

export type AgentTerminalContext = {
  workspace: string
  worktree: string
  /** Kernel workspace identity; distinct from the local filesystem path above. */
  workspace_id?: string | null
  /** Kernel worktree identity; distinct from the local filesystem path above. */
  worktree_id?: string | null
  session_id?: string | null
  attachment_id?: string | null
  agent_id?: string | null
  workflow_id?: string | null
  provider?: string
  model?: string
  effort?: string
  variables?: Record<string, string>
  targets?: Record<string, string>
}

export type AgentTerminalCommandNode = {
  id: string
  label: string
  description: string
  value: string
  kind: string
  execution_target: string
  surfaces: string[]
  search_aliases?: string[]
  intents?: string[]
  examples?: string[]
  dynamic_source?: string | null
  children?: AgentTerminalCommandNode[]
  presentation_only?: boolean
}

export type AgentTerminalCatalog = {
  revision: string
  nodes: AgentTerminalCommandNode[]
}

export type AgentTerminalSearchOptions = {
  query?: string | undefined
  limit?: number | undefined
}

export type AgentTerminalSearchResult = AgentTerminalCommandNode & {
  score: number
}

export type AgentTerminalExecution = {
  ok: boolean
  output: string
  result: unknown
  context: AgentTerminalContext
  registry_revision: string
}

export type AgentTerminalWaitResult = {
  completed: boolean
  timed_out: boolean
  session: unknown
  agent_activity?: unknown
  agent_activity_revision?: number | null
}

export type AgentTerminalStatus = {
  connected: boolean
  context: AgentTerminalContext
  session: unknown
  agent_activity?: unknown
  agent_activity_revision?: number | null
  registry_revision: string
}

export type AgentTerminalClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  subscribeToKernelEvents?: (sessionId: string, attachmentId: string) => Promise<void>
  unsubscribeFromKernelEvents?: () => Promise<void>
  onKernelEvent?: (handler: (event: KernelEvent) => void) => () => void
  close?: () => Promise<void>
}

export class AgentTerminal {
  private catalog: AgentTerminalCatalog | null = null
  private registry: TerminalOperationRegistry | null = null
  private pendingPromptWaits = new Map<string, { prompt_ids: Set<string>; revision: number | null }>()
  private attachments = new Map<string, string>()
  private ownedAttachments = new Set<string>()
  private sessionClientIds = new Map<string, string>()
  private subscribedAttachmentKey: string | null = null
  private eventClients = new Map<string, AgentTerminalClient>()
  private eventClientAttachmentIds = new Map<string, string>()
  private eventClientUnsubscribers = new Map<string, () => void>()
  private eventSubscriptionPromises = new Map<string, Promise<void>>()
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private readonly removeBaseEventHandler: (() => void) | null

  constructor(
    private readonly client: AgentTerminalClient,
    private readonly clientId = `chariox-agent-terminal-${process.pid}-${Date.now()}`,
    private readonly eventClientFactory?: (() => AgentTerminalClient) | undefined,
  ) {
    this.removeBaseEventHandler = client.onKernelEvent
      ? client.onKernelEvent((event) => this.emitKernelEvent(event))
      : null
  }

  async getCatalog(): Promise<AgentTerminalCatalog> {
    if (this.catalog) return this.catalog
    const response = await this.client.send(getTerminalCommandCatalogRequest())
    const catalog = (response.TerminalCommandCatalog as { catalog?: AgentTerminalCatalog } | undefined)?.catalog
    if (!catalog || typeof catalog.revision !== "string" || !Array.isArray(catalog.nodes)) {
      throw new Error("kernel returned an invalid terminal command catalog")
    }
    this.catalog = catalog
    return catalog
  }

  async getRegistry(options: { refresh?: boolean } = {}): Promise<TerminalOperationRegistry> {
    if (this.registry && !options.refresh) return this.registry
    try {
      const response = await this.client.send(getTerminalOperationRegistryRequest())
      const registry = (response.TerminalOperationRegistry as { registry?: TerminalOperationRegistry } | undefined)?.registry
      if (registry && typeof registry.revision === "string" && Array.isArray(registry.operations)) {
        this.registry = registry
        return registry
      }
    } catch {
      // Older kernels only expose the presentation catalog; convert it below.
    }
    const catalog = await this.getCatalog()
    const operations = flattenCatalog(catalog.nodes).map(commandToOperation)
    this.registry = { revision: catalog.revision, operations }
    return this.registry
  }

  async search(options: AgentTerminalSearchOptions = {}): Promise<{ revision: string; results: AgentTerminalSearchResult[] }> {
    const registry = await this.getRegistry()
    const query = tokenize(options.query ?? "")
    const limit = clampLimit(options.limit)
    const results = registry.operations
      .filter((operation) => isAgentTerminalOperation(operation))
      .map((operation) => ({ node: operationToCommand(operation), score: scoreOperation(operation, query) }))
      .filter(({ score }) => query.length === 0 || score > 0)
      .sort((left, right) => right.score - left.score || left.node.id.localeCompare(right.node.id))
      .slice(0, limit)
      .map(({ node, score }) => ({ ...node, score }))
    return { revision: registry.revision, results }
  }

  async describe(commandId: string): Promise<{ revision: string; command: AgentTerminalCommandNode; operation: TerminalOperationContract }> {
    const registry = await this.getRegistry()
    const operation = registry.operations.find((candidate) => candidate.id === commandId)
    if (!operation) throw new Error(`unknown terminal operation: ${commandId}`)
    if (!isAgentTerminalOperation(operation)) throw new Error(`terminal operation is not supported by agent terminals: ${commandId}`)
    return { revision: registry.revision, command: operationToCommand(operation), operation }
  }

  async status(context: AgentTerminalContext): Promise<AgentTerminalStatus> {
    const normalizedContext = normalizeContext(context)
    const effectiveContext = normalizedContext.session_id
      ? await this.contextWithAttachment(normalizedContext)
      : normalizedContext
    let session: unknown = null
    let agentActivity: unknown = null
    let agentActivityRevision: number | null = null
    if (effectiveContext.session_id) {
      const response = await this.client.send(getSessionStateRequest(effectiveContext.session_id))
      const state = sessionStatePayload(response)
      session = state?.session ?? null
      agentActivity = state?.agent_activity ?? sessionAgentActivity(session)
      agentActivityRevision = typeof state?.agent_activity_revision === "number" ? state.agent_activity_revision : null
    }
    const registry = await this.getRegistry()
    return {
      connected: true,
      context: effectiveContext,
      session,
      agent_activity: agentActivity,
      agent_activity_revision: agentActivityRevision,
      registry_revision: registry.revision,
    }
  }

  onKernelEvent(handler: (event: KernelEvent) => void): () => void {
    this.eventHandlers.add(handler)
    return () => this.eventHandlers.delete(handler)
  }

  async executeOperation(
    operationId: string,
    input: unknown,
    context: AgentTerminalContext,
    options: { write?: ((text: string) => void) | undefined; signal?: AbortSignal | undefined; shell?: ShellScriptOptions | undefined; registry_revision?: string | undefined } = {},
  ): Promise<AgentTerminalExecution> {
    const registry = await this.getRegistry({ refresh: options.registry_revision !== undefined })
    assertExpectedRegistryRevision(registry.revision, options.registry_revision)
    const operation = registry.operations.find((candidate) => candidate.id === operationId)
    if (!operation) throw new Error(`unknown terminal operation: ${operationId}`)
    if (!isAgentTerminalOperation(operation)) throw new Error(`terminal operation is not supported by agent terminals: ${operationId}`)
    if (!operation.command) {
      const variant = operation.parity_variants?.[0]
      if (!variant) throw new Error(`terminal operation has no executable adapter: ${operationId}`)
      return this.executeKernelOperation(operation, input, context, options)
    }
    const suffix = typeof input === "string" ? input : input === undefined ? "" : JSON.stringify(input)
    const command = `${operation.command}${suffix && !/[\s]$/.test(operation.command) ? " " : ""}${suffix}`
    return this.execute(command, context, options)
  }

  private async executeKernelOperation(
    operation: TerminalOperationContract,
    input: unknown,
    context: AgentTerminalContext,
    options: { signal?: AbortSignal | undefined; registry_revision?: string | undefined } = {},
  ): Promise<AgentTerminalExecution> {
    const normalizedContext = normalizeContext(context)
    const variant = operation.parity_variants?.[0]
    if (!variant) throw new Error(`terminal operation has no kernel variant: ${operation.id}`)
    let effectiveContext = normalizedContext
    if (normalizedContext.session_id && variant !== "AttachToSession" && variant !== "DetachFromSession") {
      effectiveContext = await this.contextWithAttachment(normalizedContext)
    }
    const targets = operation.required_targets ?? []
    const payload = input && typeof input === "object" ? { ...(input as Record<string, unknown>) } : {}
    for (const target of targets) {
      const value = contextValueForTarget(target, effectiveContext, operation.parity_variants?.[0])
      if (value === undefined || value === null) throw new Error(`terminal operation ${operation.id} requires explicit ${target}`)
      const inputKey = contextInputKey(target, operation.parity_variants?.[0])
      payload[inputKey] = value
    }
    if (variant === "SubmitPrompt") {
      payload.prompt_source = "agent_terminal"
    }
    if (variant === "SubmitPrompts" && !effectiveContext.agent_id) {
      throw new Error("terminal operation SubmitPrompts requires explicit agent_id context")
    }
    if (variant === "SubmitPrompts" && Array.isArray(payload.prompts)) {
      payload.prompts = payload.prompts.map((prompt) => ({
        ...(prompt as Record<string, unknown>),
        session_id: effectiveContext.session_id,
        attachment_id: effectiveContext.attachment_id,
        target_agent_id: effectiveContext.agent_id,
        prompt_source: "agent_terminal",
      }))
    }
    if (variant === "AttachToSession") {
      // The attachment identity and capability are owned by this terminal;
      // callers cannot impersonate another client or downgrade the agent
      // surface through structured input.
      payload.client_id = effectiveContext.session_id
        ? this.clientIdForSession(effectiveContext.session_id)
        : this.clientId
      payload.capability_level = "FullTerminal"
    }
    const requestValue = input === undefined && targets.length === 0
      ? (operation.input_schema?.type === "object" ? payload : null)
      : payload
    validateStructuredInput(operation.input_schema, requestValueForValidation(payload, requestValue, operation.input_schema), operation.id)
    let response: Record<string, unknown>
    try {
      response = await abortable(this.client.send({ [variant]: requestValue }), options.signal)
    } catch (error) {
      if (variant === "AttachToSession" || variant === "DetachFromSession" || !effectiveContext.session_id || !isStaleAttachmentError(error)) {
        throw error
      }
      await this.forgetAttachment(effectiveContext.attachment_id ?? "")
      effectiveContext = await this.contextWithAttachment({ ...effectiveContext, attachment_id: null })
      const retryPayload = { ...payload }
      for (const target of targets) {
        const value = contextValueForTarget(target, effectiveContext, variant)
        if (value !== undefined && value !== null) retryPayload[contextInputKey(target, variant)] = value
      }
      if (variant === "SubmitPrompts" && Array.isArray(retryPayload.prompts)) {
        retryPayload.prompts = retryPayload.prompts.map((prompt) => ({
          ...(prompt as Record<string, unknown>),
          session_id: effectiveContext.session_id,
          attachment_id: effectiveContext.attachment_id,
          target_agent_id: effectiveContext.agent_id,
          prompt_source: "agent_terminal",
        }))
      }
      const retryValue = input === undefined && targets.length === 0
        ? (operation.input_schema?.type === "object" ? retryPayload : null)
        : retryPayload
      response = await abortable(this.client.send({ [variant]: retryValue }), options.signal)
    }
    if (variant === "AttachToSession") {
      const sessionId = effectiveContext.session_id
      const attachment = (response.SessionAttached as { attachment?: { id?: string } } | undefined)?.attachment
      if (sessionId && attachment?.id) {
        this.rememberAttachment(sessionId, attachment.id)
        effectiveContext = { ...effectiveContext, attachment_id: attachment.id }
        await this.ensureEventSubscription(sessionId, attachment.id)
      }
    } else if (variant === "DetachFromSession") {
      const detachedAttachmentId = typeof payload.attachment_id === "string" ? payload.attachment_id : null
      if (detachedAttachmentId) await this.forgetAttachment(detachedAttachmentId)
      effectiveContext = { ...effectiveContext, attachment_id: null }
    }
    const safeResponse = redactSensitiveValue(response)
    if (variant === "SubmitPrompt") {
      const submitted = (response as { PromptSubmitted?: { outcome?: Record<string, unknown>; agent_activity_revision?: number } }).PromptSubmitted
      const outcome = submitted?.outcome
      const prompt = outcome && typeof outcome === "object"
        ? (Object.values(outcome)[0] as { prompt?: { id?: string } } | undefined)?.prompt
        : undefined
      const promptId = typeof prompt?.id === "string" ? prompt.id : null
      const targetAgentId = effectiveContext.agent_id
      const sessionId = effectiveContext.session_id
      if (promptId && targetAgentId && sessionId) {
        const key = promptWaitKey(sessionId, targetAgentId)
        this.pendingPromptWaits.set(key, {
          prompt_ids: new Set([promptId]),
          revision: typeof submitted?.agent_activity_revision === "number" ? submitted.agent_activity_revision : null,
        })
      }
    }
    if (variant === "SubmitPrompts") {
      const submitted = (response as { PromptsSubmitted?: { results?: unknown[]; agent_activity_revision?: number } }).PromptsSubmitted
      const revision = typeof submitted?.agent_activity_revision === "number" ? submitted.agent_activity_revision : null
      for (const result of submitted?.results ?? []) {
        if (!result || typeof result !== "object") continue
        const entry = result as { agent_id?: unknown; outcome?: unknown }
        const agentId = typeof entry.agent_id === "string" ? entry.agent_id : null
        const promptId = promptIdFromOutcome(entry.outcome)
        if (!agentId || !promptId || !effectiveContext.session_id) continue
        this.pendingPromptWaits.set(promptWaitKey(effectiveContext.session_id, agentId), {
          prompt_ids: new Set([promptId]),
          revision,
        })
      }
    }
    return {
      ok: true,
      output: JSON.stringify(safeResponse),
      result: safeResponse,
      context: effectiveContext,
      registry_revision: this.registry?.revision ?? "",
    }
  }

  async execute(
    command: string,
    context: AgentTerminalContext,
    options: { write?: ((text: string) => void) | undefined; signal?: AbortSignal | undefined; shell?: ShellScriptOptions | undefined; registry_revision?: string | undefined } = {},
  ): Promise<AgentTerminalExecution> {
    let registry: TerminalOperationRegistry | null = this.registry
    if (options.registry_revision !== undefined) {
      registry = await this.getRegistry({ refresh: true })
      assertExpectedRegistryRevision(registry.revision, options.registry_revision)
    }
    const normalizedContext = normalizeContext(context)
    validateAgentTerminalCommand(command, normalizedContext)
    const effectiveContext = normalizedContext.session_id
      ? await this.contextWithAttachment(normalizedContext)
      : normalizedContext
    const shellContext = toShellContext(effectiveContext)
    const output: string[] = []
    const shellOptions: ShellScriptOptions = {
      ...(options.shell ?? {}),
      signal: options.signal,
      echoCommands: false,
      loadScript: async (path, nestedContext) => {
        const root = await canonicalPath(nestedContext.worktree)
        const candidate = await canonicalPath(path)
        if (!isPathInsideWorktree(candidate, root)) {
          throw new Error("agent terminal scripts must stay inside the selected worktree")
        }
        if (options.shell?.loadScript) {
          return options.shell.loadScript(candidate, nestedContext)
        }
        return readFile(candidate, "utf8")
      },
      validateCommand: (line, nestedContext) => validateAgentTerminalCommand(line, fromShellContext(nestedContext)),
      redactResult: (result) => ({
        ...result,
        ...(result.data === undefined ? {} : { data: redactSensitiveValue(result.data) }),
      }),
    }
    const result = await abortable(
      executeShellLine(
        command,
        shellContext,
        { client: this.shellClient(), clientId: this.clientId, signal: options.signal },
        (text) => {
          const safeText = redactSensitiveText(text)
          output.push(safeText)
          options.write?.(safeText)
        },
        shellOptions,
      ),
      options.signal,
    )
    if (!registry) {
      try {
        registry = await this.getRegistry()
      } catch {
        // Older/lightweight clients may not expose the registry. The shell
        // command itself has already completed, so preserve compatibility and
        // report an empty revision rather than masking its result.
      }
    }
    const resultContext = fromShellContext(result.context)
    if (resultContext.session_id) {
      const replacementAttachment = this.attachments.get(resultContext.session_id)
      if (replacementAttachment) resultContext.attachment_id = replacementAttachment
    }
    return {
      ok: result.ok,
      output: output.join(""),
      result: result.context,
      context: { ...resultContext, targets: { ...(effectiveContext.targets ?? {}) } },
      registry_revision: registry?.revision ?? "",
    }
  }

  async wait(context: AgentTerminalContext, timeoutMs = 120_000, signal?: AbortSignal): Promise<AgentTerminalWaitResult> {
    const normalizedContext = normalizeContext(context)
    if (!normalizedContext.session_id || !normalizedContext.agent_id) {
      throw new Error("agent terminal wait requires explicit session_id and agent_id")
    }
    const effectiveContext = await this.contextWithAttachment(normalizedContext)
    const boundedTimeout = timeoutMs === undefined || !Number.isFinite(timeoutMs)
      ? 120_000
      : Math.min(Math.max(Math.trunc(timeoutMs), 0), 120_000)
    const deadline = Date.now() + boundedTimeout
    let session: unknown = null
    let agentActivity: unknown = null
    let agentActivityRevision: number | null = null
    do {
      throwIfAborted(signal)
      await this.client.send(pumpTerminalOutputRequest(effectiveContext.session_id!, effectiveContext.attachment_id!)).catch(() => ({}))
      const response = await this.client.send(getSessionStateRequest(effectiveContext.session_id!))
      throwIfAborted(signal)
      const state = sessionStatePayload(response)
      session = state?.session ?? null
      agentActivity = state?.agent_activity ?? sessionAgentActivity(session)
      agentActivityRevision = typeof state?.agent_activity_revision === "number" ? state.agent_activity_revision : null
      const pendingKey = promptWaitKey(effectiveContext.session_id, effectiveContext.agent_id)
      const pending = this.pendingPromptWaits.get(pendingKey)
      if (pending && pending.revision !== null && (agentActivityRevision === null || agentActivityRevision <= pending.revision)) {
        // Do not trust an older projection after this terminal accepted work.
      } else if (isIdle(agentActivity, normalizedContext.agent_id)) {
        this.pendingPromptWaits.delete(pendingKey)
        return { completed: true, timed_out: false, session, agent_activity: agentActivity, agent_activity_revision: agentActivityRevision }
      }
      if (Date.now() >= deadline) break
      await sleep(Math.min(250, Math.max(deadline - Date.now(), 0)), signal)
    } while (Date.now() <= deadline)
    return { completed: false, timed_out: true, session, agent_activity: agentActivity, agent_activity_revision: agentActivityRevision }
  }

  async close(): Promise<void> {
    const eventClients = [...this.eventClients.entries()]
    this.eventClients.clear()
    this.eventClientAttachmentIds.clear()
    this.eventSubscriptionPromises.clear()
    await Promise.all(eventClients.map(async ([key, eventClient]) => {
      this.eventClientUnsubscribers.get(key)?.()
      this.eventClientUnsubscribers.delete(key)
      await eventClient.unsubscribeFromKernelEvents?.().catch(() => {})
      await eventClient.close?.().catch(() => {})
    }))
    this.eventClientUnsubscribers.clear()
    if (this.client.unsubscribeFromKernelEvents) await this.client.unsubscribeFromKernelEvents().catch(() => {})
    this.removeBaseEventHandler?.()
    this.subscribedAttachmentKey = null
    const owned = [...this.ownedAttachments]
    this.ownedAttachments.clear()
    this.attachments.clear()
    this.sessionClientIds.clear()
    await Promise.all(owned.map((attachmentId) => this.client.send(detachFromSessionRequest(attachmentId)).catch(() => ({}))))
  }

  private async contextWithAttachment(context: AgentTerminalContext): Promise<AgentTerminalContext> {
    if (!context.session_id) return context
    if (context.attachment_id) {
      this.attachments.set(context.session_id, context.attachment_id)
      await this.ensureEventSubscription(context.session_id, context.attachment_id)
      return context
    }
    const existing = this.attachments.get(context.session_id)
    if (existing) return { ...context, attachment_id: existing }
    const response = await this.client.send(attachToSessionRequest(context.session_id, this.clientIdForSession(context.session_id)))
    const attachment = (response.SessionAttached as { attachment?: { id?: string } } | undefined)?.attachment
    if (!attachment?.id) throw new Error(`kernel did not return an attachment for session ${context.session_id}`)
    this.rememberAttachment(context.session_id, attachment.id)
    await this.ensureEventSubscription(context.session_id, attachment.id)
    return { ...context, attachment_id: attachment.id }
  }

  private shellClient(): { send: (request: Record<string, unknown>) => Promise<Record<string, unknown>> } {
    return {
      send: async (request) => {
        const attach = request.AttachToSession as { session_id?: unknown } | undefined
        if (attach && typeof attach.session_id === "string") {
          const response = await this.client.send({
            ...request,
            AttachToSession: {
              ...attach,
              client_id: this.clientIdForSession(attach.session_id),
            },
          })
          const attachment = (response.SessionAttached as { attachment?: { id?: string } } | undefined)?.attachment
          if (attachment?.id) {
            this.rememberAttachment(attach.session_id, attachment.id)
            await this.ensureEventSubscription(attach.session_id, attachment.id)
          }
          return response
        }
        const detach = request.DetachFromSession as { attachment_id?: unknown } | undefined
        let detachedAttachmentId = typeof detach?.attachment_id === "string" ? detach.attachment_id : null
        let response: Record<string, unknown>
        try {
          response = await this.client.send(request)
        } catch (error) {
          if (!isStaleAttachmentError(error)) throw error
          const entry = Object.entries(request).find(([, value]) => value && typeof value === "object" && !Array.isArray(value))
          const payload = entry?.[1] as Record<string, unknown> | undefined
          const sessionId = typeof payload?.session_id === "string" ? payload.session_id : null
          const attachmentId = typeof payload?.attachment_id === "string" ? payload.attachment_id : null
          if (!entry || !sessionId || !attachmentId) throw error
          await this.forgetAttachment(attachmentId)
          const replacement = await this.contextWithAttachment({ workspace: "", worktree: "", session_id: sessionId, attachment_id: null })
          if (detachedAttachmentId) detachedAttachmentId = replacement.attachment_id ?? detachedAttachmentId
          response = await this.client.send({
            [entry[0]]: {
              ...payload,
              attachment_id: replacement.attachment_id,
            },
          })
        }
        if (detachedAttachmentId) {
          await this.forgetAttachment(detachedAttachmentId)
        }
        return response
      },
    }
  }

  private clientIdForSession(sessionId: string): string {
    const existing = this.sessionClientIds.get(sessionId)
    if (existing) return existing
    const suffix = sessionId.replace(/[^A-Za-z0-9_-]/g, "_")
    const clientId = `${this.clientId}:${suffix}`
    this.sessionClientIds.set(sessionId, clientId)
    return clientId
  }

  private rememberAttachment(sessionId: string, attachmentId: string): void {
    const previous = this.attachments.get(sessionId)
    if (previous && previous !== attachmentId) this.ownedAttachments.delete(previous)
    this.attachments.set(sessionId, attachmentId)
    this.ownedAttachments.add(attachmentId)
  }

  private async forgetAttachment(attachmentId: string): Promise<void> {
    const eventClientEntries: [string, AgentTerminalClient][] = []
    for (const [sessionId, cachedAttachmentId] of this.attachments) {
      if (cachedAttachmentId === attachmentId) {
        this.attachments.delete(sessionId)
        const eventClient = this.eventClients.get(sessionId)
        if (eventClient) eventClientEntries.push([sessionId, eventClient])
        this.eventClients.delete(sessionId)
        this.eventClientAttachmentIds.delete(sessionId)
        this.eventSubscriptionPromises.delete(sessionId)
      }
    }
    this.ownedAttachments.delete(attachmentId)
    await Promise.all(eventClientEntries.map(async ([sessionId, eventClient]) => {
      this.eventClientUnsubscribers.get(sessionId)?.()
      this.eventClientUnsubscribers.delete(sessionId)
      await eventClient.unsubscribeFromKernelEvents?.().catch(() => {})
      await eventClient.close?.().catch(() => {})
    }))
    if (this.subscribedAttachmentKey?.endsWith(`:${attachmentId}`)) {
      await this.client.unsubscribeFromKernelEvents?.().catch(() => {})
      this.subscribedAttachmentKey = null
    }
  }

  private async ensureEventSubscription(sessionId: string, attachmentId: string): Promise<void> {
    if (this.eventClientFactory) {
      const existing = this.eventClients.get(sessionId)
      if (existing && this.eventClientAttachmentIds.get(sessionId) === attachmentId) return
      if (existing) await this.closeDedicatedEventClient(sessionId, existing)
      const pending = this.eventSubscriptionPromises.get(sessionId)
      if (pending) return pending
      const subscription = this.startDedicatedEventSubscription(sessionId, attachmentId)
      this.eventSubscriptionPromises.set(sessionId, subscription)
      try {
        await subscription
      } finally {
        this.eventSubscriptionPromises.delete(sessionId)
      }
      return
    }
    if (!this.client.subscribeToKernelEvents) return
    const key = `${sessionId}:${attachmentId}`
    if (this.subscribedAttachmentKey === key) return
    await this.client.subscribeToKernelEvents(sessionId, attachmentId)
    this.subscribedAttachmentKey = key
  }

  private async startDedicatedEventSubscription(sessionId: string, attachmentId: string): Promise<void> {
    const eventClient = this.eventClientFactory?.()
    if (!eventClient?.subscribeToKernelEvents) return
    const removeHandler = eventClient.onKernelEvent?.((event) => this.emitKernelEvent(event)) ?? (() => {})
    try {
      await eventClient.subscribeToKernelEvents(sessionId, attachmentId)
      this.eventClients.set(sessionId, eventClient)
      this.eventClientAttachmentIds.set(sessionId, attachmentId)
      this.eventClientUnsubscribers.set(sessionId, removeHandler)
    } catch (error) {
      removeHandler()
      await eventClient.close?.().catch(() => {})
      throw error
    }
  }

  private async closeDedicatedEventClient(sessionId: string, eventClient: AgentTerminalClient): Promise<void> {
    this.eventClientUnsubscribers.get(sessionId)?.()
    this.eventClientUnsubscribers.delete(sessionId)
    this.eventClients.delete(sessionId)
    this.eventClientAttachmentIds.delete(sessionId)
    this.eventSubscriptionPromises.delete(sessionId)
    await eventClient.unsubscribeFromKernelEvents?.().catch(() => {})
    await eventClient.close?.().catch(() => {})
  }

  private emitKernelEvent(event: KernelEvent): void {
    for (const handler of this.eventHandlers) handler(event)
  }
}

export function normalizeContext(context: AgentTerminalContext): AgentTerminalContext {
  if (!context || typeof context !== "object") throw new Error("agent terminal context is required")
  if (!context.workspace?.trim() || !context.worktree?.trim()) {
    throw new Error("agent terminal context requires workspace and worktree")
  }
  return {
    workspace: context.workspace,
    worktree: context.worktree,
    workspace_id: context.workspace_id?.trim() || null,
    worktree_id: context.worktree_id?.trim() || null,
    session_id: context.session_id?.trim() || null,
    attachment_id: context.attachment_id?.trim() || null,
    agent_id: context.agent_id?.trim() || null,
    workflow_id: context.workflow_id?.trim() || null,
    provider: context.provider?.trim() || "opencode",
    model: context.model?.trim() || "default",
    effort: context.effort?.trim() || "medium",
    variables: { ...(context.variables ?? {}) },
    targets: { ...(context.targets ?? {}) },
  }
}

function validateAgentTerminalCommand(command: string, context: AgentTerminalContext): void {
  const parsed = parseShellCommand(command, { variables: context.variables ?? {} })
  if (parsed.kind === "empty" || parsed.kind === "invalid") throw new Error(parsed.reason ?? "command is required")
  if (parsed.kind === "tui-only") throw new Error(parsed.reason ?? "command is not available to agent terminals")
  if (parsed.command === "agent" || parsed.command === "agents") {
    const action = parsed.args[0]?.toLowerCase()
    if (action === "focus" || action === "cycle") {
      throw new Error("agent terminals cannot change human focus; target agents explicitly")
    }
  }
  if (parsed.command === "prompt") {
    if (!context.session_id) throw new Error("agent terminal prompt commands require an explicit session_id")
    if (!context.agent_id) throw new Error("agent terminal prompt commands require an explicit agent_id")
  }
}

function toShellContext(context: AgentTerminalContext): ShellContext {
  return createDefaultShellContext({
    workspace: context.workspace,
    worktree: context.worktree,
    workspaceId: context.workspace_id ?? undefined,
    worktreeId: context.worktree_id ?? undefined,
    sessionId: context.session_id ?? undefined,
    attachmentId: context.attachment_id ?? undefined,
    agentId: context.agent_id ?? undefined,
    workflowId: context.workflow_id ?? undefined,
    provider: context.provider ?? "opencode",
    model: context.model ?? "default",
    effort: context.effort ?? "medium",
    promptSource: "agent_terminal",
    ...(context.variables ? { variables: context.variables } : {}),
  })
}

function fromShellContext(context: ShellContext): AgentTerminalContext {
  return {
    workspace: context.workspace,
    worktree: context.worktree,
    workspace_id: context.workspaceId ?? null,
    worktree_id: context.worktreeId ?? null,
    session_id: context.sessionId ?? null,
    attachment_id: context.attachmentId ?? null,
    agent_id: context.agentId ?? null,
    workflow_id: context.workflowId ?? null,
    provider: context.provider,
    model: context.model,
    effort: context.effort,
    variables: { ...context.variables },
  }
}

function flattenCatalog(nodes: AgentTerminalCommandNode[], out: AgentTerminalCommandNode[] = []): AgentTerminalCommandNode[] {
  for (const node of nodes) {
    out.push({ ...node, presentation_only: isPresentationOnly(node) })
    if (node.children) flattenCatalog(node.children, out)
  }
  return out
}

const HUMAN_PRESENTATION_COMMANDS = new Set(["agent-focus", "agent-cycle", "view", "view-split", "view-individual", "waiting", "exit", "quit", "credential-set", "credential-register"])

function isPresentationOnly(node: AgentTerminalCommandNode): boolean {
  return node.execution_target !== "kernel" || HUMAN_PRESENTATION_COMMANDS.has(node.id)
}

function tokenize(value: string): string[] {
  return value.toLowerCase().trim().split(/[^a-z0-9_-]+/).filter(Boolean)
}

function scoreNode(node: AgentTerminalCommandNode, query: string[]): number {
  if (query.length === 0) return 1
  const haystack = tokenize([
    node.id,
    node.label,
    node.description,
    ...(node.search_aliases ?? []),
    ...(node.intents ?? []),
  ].join(" "))
  return query.reduce((score, term) => score + bestTokenScore(term, haystack, node.id), 0)
}

function scoreOperation(operation: TerminalOperationContract, query: string[]): number {
  if (query.length === 0) return 1
  const haystack = tokenize([
    operation.id,
    operation.description,
    ...(operation.search_aliases ?? []),
    ...(operation.intents ?? []),
  ].join(" "))
  return query.reduce((score, term) => score + bestTokenScore(term, haystack, operation.id), 0)
}

function bestTokenScore(term: string, haystack: string[], id: string): number {
  if (haystack.includes(term)) return id === term ? 10 : 1
  if (haystack.some((candidate) => candidate.startsWith(term))) return 0.5
  if (term.length < 4) return 0
  const maxDistance = term.length >= 8 ? 2 : 1
  return haystack.some((candidate) => candidate.length >= 4 && levenshteinDistanceAtMost(term, candidate, maxDistance) <= maxDistance)
    ? 0.25
    : 0
}

function levenshteinDistanceAtMost(left: string, right: string, limit: number): number {
  if (Math.abs(left.length - right.length) > limit) return limit + 1
  let previous = Array.from({ length: right.length + 1 }, (_, index) => index)
  for (let leftIndex = 1; leftIndex <= left.length; leftIndex += 1) {
    const current: number[] = [leftIndex]
    let rowMinimum = leftIndex
    for (let rightIndex = 1; rightIndex <= right.length; rightIndex += 1) {
      const cost = left[leftIndex - 1] === right[rightIndex - 1] ? 0 : 1
      const value = Math.min(
        (current[rightIndex - 1] ?? limit + 1) + 1,
        (previous[rightIndex] ?? limit + 1) + 1,
        (previous[rightIndex - 1] ?? limit + 1) + cost,
      )
      current[rightIndex] = value
      rowMinimum = Math.min(rowMinimum, value)
    }
    if (rowMinimum > limit) return limit + 1
    previous = current
  }
  return previous[right.length] ?? limit + 1
}

function contextValueForTarget(target: string, context: AgentTerminalContext, variant: string | undefined): string | null | undefined {
  if (target === "agent_id" || target === "target_agent_id") return context.agent_id ?? context.targets?.[target]
  if (target === "workflow_id" || target === "workflow_ref") return context.workflow_id ?? context.targets?.[target]
  if (target === "session_id") return context.session_id ?? context.targets?.[target]
  if (target === "attachment_id") return context.attachment_id ?? context.targets?.[target]
  if (target === "workspace_id") return context.workspace_id ?? context.targets?.[target]
  if (target === "worktree_id") return context.worktree_id ?? context.targets?.[target]
  return context.targets?.[target]
}

function contextInputKey(target: string, variant: string | undefined): string {
  if (target === "agent_id" && variant === "SubmitPrompt") return "target_agent_id"
  return target
}

function commandToOperation(command: AgentTerminalCommandNode): TerminalOperationContract {
  return {
    id: command.id,
    command: command.value,
    description: command.description,
    ...(command.search_aliases ? { search_aliases: command.search_aliases } : {}),
    ...(command.intents ? { intents: command.intents } : {}),
    required_context: ["workspace", "worktree"],
    required_targets: [],
    input_schema: { type: "string", description: "The native Chariox terminal command arguments" },
    result_kind: "terminal_command",
    mutation: !terminalCommandReadAction(command.id),
    expected_projections: ["session_snapshot", "terminal_events"],
    supported_surfaces: command.execution_target === "kernel" && !isPresentationOnly(command)
      ? [...command.surfaces, "agent_terminal"]
      : command.surfaces,
    ...(command.examples ? { examples: command.examples } : {}),
    parity_variants: [],
    presentation_only: isPresentationOnly(command),
  }
}

function operationToCommand(operation: TerminalOperationContract): AgentTerminalCommandNode {
  return {
    id: operation.id,
    label: operation.id.replace(/^terminal\./, "").replaceAll("_", " "),
    description: operation.description,
    value: operation.command ?? operation.id,
    kind: "command",
    execution_target: operation.presentation_only ? "terminal-local" : "kernel",
    surfaces: operation.supported_surfaces ?? ["session"],
    ...(operation.search_aliases ? { search_aliases: operation.search_aliases } : {}),
    ...(operation.intents ? { intents: operation.intents } : {}),
    ...(operation.examples ? { examples: operation.examples } : {}),
    presentation_only: operation.presentation_only,
  }
}

function terminalCommandReadAction(id: string): boolean {
  const action = id.split("-").at(-1) ?? id
  return new Set([
    "get", "list", "show", "read", "search", "inspect", "open", "status", "health",
    "members", "collaborators", "invites", "kernels", "ls", "runs", "logs", "trace", "preview", "validate",
  ]).has(action)
}

function isAgentTerminalOperation(operation: TerminalOperationContract): boolean {
  return !operation.presentation_only && (operation.supported_surfaces ?? []).includes("agent_terminal")
}

function clampLimit(limit: number | undefined): number {
  if (limit === undefined || !Number.isFinite(limit)) return 20
  return Math.min(Math.max(Math.trunc(limit), 1), 50)
}

function assertExpectedRegistryRevision(actual: string, expected: string | undefined): void {
  if (expected !== undefined && expected !== actual) {
    throw new Error(`terminal operation registry revision mismatch: expected ${expected}, current ${actual}`)
  }
}

type AgentActivity = {
  busy?: boolean
  status?: string
  prompt_status?: string
  active_prompt_count?: number
  queued_prompt_count?: number
  active_turn?: { status?: string; phase?: string } | null
}

type SessionStatePayload = {
  session?: unknown
  agent_activity?: Record<string, AgentActivity>
  agent_activity_revision?: number
}

function sessionStatePayload(response: Record<string, unknown>): SessionStatePayload | null {
  const state = response.SessionState ?? response.SessionStateLoaded
  return state && typeof state === "object" ? state as SessionStatePayload : null
}

function sessionAgentActivity(session: unknown): Record<string, AgentActivity> | null {
  if (!session || typeof session !== "object") return null
  const activity = (session as { agent_activity?: unknown }).agent_activity
  return activity && typeof activity === "object" ? activity as Record<string, AgentActivity> : null
}

function isIdle(activitySource: unknown, agentId: string | null | undefined): boolean {
  if (!activitySource || typeof activitySource !== "object") return false
  const value = activitySource as { agent_activity?: Record<string, AgentActivity> } & Record<string, unknown>
  if (!agentId) return false
  const activity = value.agent_activity?.[agentId] ?? (value[agentId] as AgentActivity | undefined)
  if (!activity || typeof activity !== "object") return false
  if (activity.busy || activity.status === "working") return false
  if (activity.active_prompt_count !== undefined && activity.active_prompt_count > 0) return false
  if (activity.queued_prompt_count !== undefined && activity.queued_prompt_count > 0) return false
  if (activity.prompt_status && activity.prompt_status !== "none") return false
  if (activity.active_turn && activity.active_turn.status !== "none") return false
  return true
}

function promptWaitKey(sessionId: string | null | undefined, agentId: string | null | undefined): string {
  return `${sessionId ?? ""}:${agentId ?? ""}`
}

function promptIdFromOutcome(outcome: unknown): string | null {
  if (!outcome || typeof outcome !== "object") return null
  const value = outcome as Record<string, unknown>
  for (const entry of Object.values(value)) {
    if (!entry || typeof entry !== "object") continue
    const prompt = (entry as { prompt?: unknown }).prompt
    if (prompt && typeof prompt === "object" && typeof (prompt as { id?: unknown }).id === "string") {
      return (prompt as { id: string }).id
    }
  }
  return null
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort)
      resolve()
    }, ms)
    function onAbort() {
      clearTimeout(timer)
      signal?.removeEventListener("abort", onAbort)
      reject(new DOMException("The operation was aborted", "AbortError"))
    }
    signal?.addEventListener("abort", onAbort, { once: true })
    if (signal?.aborted) onAbort()
  })
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw new DOMException("The operation was aborted", "AbortError")
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) return promise
  throwIfAborted(signal)
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      signal.addEventListener("abort", () => reject(new DOMException("The operation was aborted", "AbortError")), { once: true })
    }),
  ])
}

function isPathInsideWorktree(path: string, worktree: string): boolean {
  const root = resolvePath(worktree)
  const candidate = resolvePath(path)
  const relativePath = relative(root, candidate)
  return relativePath === "" || (!relativePath.startsWith("..") && !isAbsolute(relativePath))
}

function requestValueForValidation(payload: Record<string, unknown>, requestValue: unknown, schema: Record<string, unknown> | null | undefined): unknown {
  if (requestValue === null && schema?.type === "object") return payload
  return requestValue
}

function validateStructuredInput(schema: Record<string, unknown> | null | undefined, value: unknown, operationId: string): void {
  if (!schema || Object.keys(schema).length === 0) return
  if (Array.isArray(schema.oneOf)) {
    const failures: string[] = []
    for (const branch of schema.oneOf) {
      try {
        validateStructuredInput(branch as Record<string, unknown>, value, operationId)
        return
      } catch (error) {
        failures.push(error instanceof Error ? error.message : String(error))
      }
    }
    throw new Error(`invalid input for ${operationId}: no schema variant matched${failures[0] ? ` (${failures[0]})` : ""}`)
  }
  const type = schema.type
  if (type === "null") {
    if (value !== null) throw new Error(`invalid input for ${operationId}: expected null`)
    return
  }
  if (type === "object") {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`invalid input for ${operationId}: expected object`)
    const object = value as Record<string, unknown>
    const required = Array.isArray(schema.required) ? schema.required : []
    for (const field of required) {
      if (typeof field === "string" && (object[field] === undefined || object[field] === null)) {
        throw new Error(`invalid input for ${operationId}: missing ${field}`)
      }
    }
    const properties = schema.properties && typeof schema.properties === "object" ? schema.properties as Record<string, Record<string, unknown>> : {}
    if (schema.additionalProperties === false) {
      for (const key of Object.keys(object)) {
        if (!Object.prototype.hasOwnProperty.call(properties, key)) throw new Error(`invalid input for ${operationId}: unknown field ${key}`)
      }
    } else if (schema.additionalProperties && typeof schema.additionalProperties === "object") {
      for (const [key, entry] of Object.entries(object)) {
        if (!Object.prototype.hasOwnProperty.call(properties, key)) {
          validateStructuredInput(schema.additionalProperties as Record<string, unknown>, entry, `${operationId}.${key}`)
        }
      }
    }
    for (const [key, propertySchema] of Object.entries(properties)) {
      if (object[key] !== undefined) validateStructuredInput(propertySchema, object[key], `${operationId}.${key}`)
    }
    return
  }
  if (type === "array") {
    if (!Array.isArray(value)) throw new Error(`invalid input for ${operationId}: expected array`)
    if (schema.items && typeof schema.items === "object") {
      for (const item of value) validateStructuredInput(schema.items as Record<string, unknown>, item, operationId)
    }
    return
  }
  if (type === "string" && typeof value !== "string") throw new Error(`invalid input for ${operationId}: expected string`)
  if (type === "number" && (typeof value !== "number" || !Number.isFinite(value))) throw new Error(`invalid input for ${operationId}: expected number`)
  if (type === "integer" && (typeof value !== "number" || !Number.isInteger(value))) throw new Error(`invalid input for ${operationId}: expected integer`)
  if (type === "boolean" && typeof value !== "boolean") throw new Error(`invalid input for ${operationId}: expected boolean`)
  if (Array.isArray(schema.enum) && !schema.enum.some((candidate) => candidate === value)) {
    throw new Error(`invalid input for ${operationId}: unsupported value`)
  }
  if (schema.const !== undefined && value !== schema.const) throw new Error(`invalid input for ${operationId}: ${String(schema.const)} is required`)
}

function redactSensitiveValue(value: unknown, path: string[] = []): unknown {
  if (Array.isArray(value)) return value.map((entry) => redactSensitiveValue(entry, path))
  if (!value || typeof value !== "object") return value
  const object = value as Record<string, unknown>
  return Object.fromEntries(Object.entries(object).map(([key, entry]) => {
    const normalizedKey = key.toLowerCase().replaceAll("-", "_")
    const sensitiveKey = /(?:^|_)(?:secret|token|password|api_key|access_key|refresh_token|private_key)(?:$|_)/.test(normalizedKey)
      || (normalizedKey === "value" && path.some((segment) => /credential|injection|vault|secret/i.test(segment)))
    return [key, sensitiveKey ? "[REDACTED]" : redactSensitiveValue(entry, [...path, key])]
  }))
}

function isStaleAttachmentError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false
  const value = error as { code?: unknown; message?: unknown }
  if (value.code === "attachment_not_found" || value.code === "attachment_not_in_session") return true
  const message = typeof value.message === "string" ? value.message.toLowerCase() : ""
  return message.includes("attachment_not_found") || message.includes("attachment_not_in_session")
}

export function redactSensitiveText(text: string): string {
  return text.replace(
    /((?:["']?(?:secret|token|password|api[_-]?key|access[_-]?key|refresh[_-]?token|private[_-]?key|cloud_invite|local_invite)["']?)\s*[:=]\s*)(["']?)([^\s,}\]"']+)(\2)/gi,
    "$1$2[REDACTED]$4",
  )
}

async function canonicalPath(path: string): Promise<string> {
  try {
    return await realpath(path)
  } catch {
    return resolvePath(path)
  }
}
