import http, { type IncomingMessage, type ServerResponse } from "node:http"
import { Readable } from "node:stream"

import {
  type RuntimeProviderRun,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  cancelActivePromptRequest,
  submitPromptRequest,
  updateProviderRunSelectionRequest,
} from "../ipc-requests.js"
import {
  preparePromptAttachmentsForSubmit,
} from "../prompt-attachment-transfer.js"
import { hiddenInstructionsStart } from "./hidden-instructions.js"
import {
  createHiddenInstructionRedactor,
  createSseHiddenInstructionRedactor,
  proxyOpenCodeEventsForNativeTui,
  requestHeadersForFetch,
} from "./opencode-event-stream.js"
import {
  openCodeNativePermissionChoice,
  resolveActiveOpenCodePermissionInteraction,
} from "./opencode-permission.js"
import {
  compactSelection,
  extractOpenCodeSelection,
  extractPromptAttachments,
  extractPromptText,
  type NativeProviderSelection,
  type OpenCodePromptBody,
  shouldApplyNativeSelection,
} from "./opencode-prompt.js"
import { getNativeProviderRun } from "./provider-run-control.js"

type OpenCodeProxyDebug = (label: string, payload: unknown) => void

type OpenCodeProxyState = {
  providerRunId: string | null
  providerSessionId: string | null
  providerRunLocal: boolean
  lastNativeSelection: NativeProviderSelection | null
}

type OpenCodeProxyOptions = {
  upstreamBaseUrl: string
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  inlineLocalAttachments: boolean
  debug: OpenCodeProxyDebug
}

export type OpenCodeNativeProxy = {
  server: http.Server
  bindProviderRun: (run: RuntimeProviderRun) => void
}

export async function startOpenCodeProxy(options: OpenCodeProxyOptions): Promise<OpenCodeNativeProxy> {
  const state: OpenCodeProxyState = {
    providerRunId: null,
    providerSessionId: null,
    providerRunLocal: true,
    lastNativeSelection: null,
  }
  const server = http.createServer((request, response) => {
    handleProxyRequest(request, response, options, state).catch((error) => {
      if (!response.headersSent) {
        response.writeHead(502, { "content-type": "application/json" })
      }
      response.end(JSON.stringify({ error: formatError(error) }))
    })
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject)
      options.debug("proxy_listening", { agentId: options.agentId })
      resolve()
    })
  })
  return {
    server,
    bindProviderRun(run) {
      state.providerRunId = run.id
      state.providerSessionId = run.provider_session_id ?? null
      state.providerRunLocal = run.session_id === options.sessionId
    },
  }
}

async function handleProxyRequest(
  request: IncomingMessage,
  response: ServerResponse,
  options: OpenCodeProxyOptions,
  state: OpenCodeProxyState,
): Promise<void> {
  const method = request.method ?? "GET"
  const path = request.url ?? "/"
  const isKernelRequest = request.headers["x-arroba-provider-client"] === "kernel"
  options.debug("request", { method, path })
  const promptMatch = method === "POST" ? path.match(/^\/session\/([^/]+)\/(?:message|prompt_async)(?:\?.*)?$/) : null
  if (promptMatch && isKernelRequest) {
    await proxyToOpenCode(request, response, options.upstreamBaseUrl, options.debug, false)
    return
  }
  if (promptMatch) {
    if (!state.providerRunId) {
      response.writeHead(503, { "content-type": "application/json" })
      response.end(JSON.stringify({ error: "OpenCode provider run is not bound yet" }))
      return
    }
    const body = await readRequestJson<OpenCodePromptBody>(request)
    options.debug(path.includes("/prompt_async") ? "prompt_async" : "message", body)
    const prompt = extractPromptText(body)
    const attachments = await preparePromptAttachmentsForSubmit(extractPromptAttachments(body), {
      inlineLocalFiles: options.inlineLocalAttachments,
    })
    if (attachments.length > 0) {
      options.debug("native_prompt_attachments_observed", {
        path,
        attachmentCount: attachments.length,
      })
    }
    const selection = extractOpenCodeSelection(body)
    if (selection) {
      options.debug("selection_observed", selection)
    }
    const run = state.providerRunLocal
      ? await getNativeProviderRun(options.client, state.providerRunId)
      : null
    if (selection && run && shouldApplyNativeSelection(selection, state.lastNativeSelection, run)) {
      const updated = await updateProviderRunSelection(
        options.client,
        options.sessionId,
        state.providerRunId,
        selection,
      )
      options.debug("selection_applied", {
        model: updated.model,
        variant: updated.variant,
      })
    } else if (selection && run) {
      options.debug("selection_ignored_as_stale", {
        incoming: selection,
        current: {
          model: run.model,
          variant: run.variant,
        },
      })
    } else if (selection) {
      options.debug("selection_observed_on_remote_run", {
        model: selection.model,
        variant: selection.variant,
      })
    }
    if (selection?.model || selection?.variant) {
      state.lastNativeSelection = compactSelection(selection)
    }
    await options.client.send<Record<string, unknown>>(
      submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
    )
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  const abortMatch = method === "POST" ? path.match(/^\/session\/([^/]+)\/abort(?:\?.*)?$/) : null
  if (abortMatch && !isKernelRequest) {
    await options.client.send<Record<string, unknown>>(
      cancelActivePromptRequest(options.sessionId, options.attachmentId),
    )
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  if (!isKernelRequest && method === "POST" && /^\/session\/[^/]+\/permissions\//.test(path)) {
    const body = await readRequestJson<Record<string, unknown>>(request).catch(() => ({}))
    const choiceId = openCodeNativePermissionChoice(body)
    const resolved = await resolveActiveOpenCodePermissionInteraction(options.client, options.sessionId, options.agentId, choiceId)
    if (!resolved) {
      response.writeHead(409, { "content-type": "application/json" })
      response.end(JSON.stringify({
        error: "No active Arroba permission interaction is available for this OpenCode native TUI response.",
      }))
      return
    }
    response.writeHead(204, { "content-length": "0" })
    response.end()
    return
  }

  if (!isKernelRequest && method === "GET" && new URL(path, options.upstreamBaseUrl).pathname === "/event") {
    await proxyOpenCodeEventsForNativeTui(request, response, options.upstreamBaseUrl, state, options.debug)
    return
  }

  await proxyToOpenCode(request, response, options.upstreamBaseUrl, options.debug, !isKernelRequest)
}

async function proxyToOpenCode(
  request: IncomingMessage,
  response: ServerResponse,
  upstreamBaseUrl: string,
  debug: OpenCodeProxyDebug,
  redactForNativeTui = false,
): Promise<void> {
  const method = request.method ?? "GET"
  const target = new URL(request.url ?? "/", upstreamBaseUrl)
  const headers = requestHeadersForFetch(request)
  const init: RequestInit = {
    method,
    headers,
  }
  if (method !== "GET" && method !== "HEAD") {
    const body = await readRequestBuffer(request)
    if (body.includes(hiddenInstructionsStart)) {
      debug("hidden_instructions_forwarded", { method, path: request.url ?? "/" })
    }
    if (body.includes("\"system\"")) {
      debug("system_context_forwarded", { method, path: request.url ?? "/" })
    }
    if (body.includes("\"type\":\"file\"") || body.includes("\"type\": \"file\"")) {
      debug("attachments_forwarded", { method, path: request.url ?? "/" })
    }
    init.body = body as unknown as BodyInit
  }
  const upstream = await fetch(target, init)

  response.statusCode = upstream.status
  upstream.headers.forEach((value, key) => {
    const lowerKey = key.toLowerCase()
    if (lowerKey !== "content-encoding" && lowerKey !== "content-length") {
      response.setHeader(key, value)
    }
  })
  if (!upstream.body) {
    response.end()
    return
  }
  await new Promise<void>((resolve, reject) => {
    const stream = Readable.fromWeb(upstream.body as never)
    const readable = redactForNativeTui
      ? stream.pipe(target.pathname === "/event"
        ? createSseHiddenInstructionRedactor()
        : createHiddenInstructionRedactor())
      : stream
    readable
      .once("error", reject)
      .once("end", resolve)
      .pipe(response)
  })
}

async function updateProviderRunSelection(
  client: LocalIpcClient,
  sessionId: string,
  providerRunId: string,
  selection: NativeProviderSelection,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(
    updateProviderRunSelectionRequest(sessionId, providerRunId, {
      model: selection.model ?? null,
      variant: selection.variant ?? null,
      clearVariant: selection.variant === "",
    }),
  )
  return expectVariant<{ provider_run: RuntimeProviderRun }>(
    response,
    "ProviderRunSelectionUpdated",
  ).provider_run
}

async function readRequestJson<T>(request: IncomingMessage): Promise<T> {
  const body = await readRequestBuffer(request)
  if (body.length === 0) {
    return {} as T
  }
  return JSON.parse(body.toString("utf8")) as T
}

async function readRequestBuffer(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = []
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
  }
  return Buffer.concat(chunks)
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
