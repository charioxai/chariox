import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"

import { runSelkiesProviderAcceptance } from "./managed-browser-computer-parity-providers.mjs"

const OPERATOR_ENDPOINT_ENV = "CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL"
const TARGET_KERNEL_ENV = "CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF"
const TARGET_MACHINE_ENV = "CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF"
const OPERATOR_CLIENT_ENV = "CHARIOX_MANAGED_PARITY_CLIENT_ID"
const OPERATOR_SESSION_ENV = "CHARIOX_MANAGED_PARITY_SESSION_ID"
const DISPLAY_CONNECT_TIMEOUT_MS = 10_000
const DISPLAY_READY_TIMEOUT_MS = 10_000

/**
 * Build the reviewed parity transport from the released public kernel client.
 *
 * The operator supplies only non-secret target references here. LocalIpcClient
 * consumes the normal one-shot local-auth configuration itself, resolves a
 * scoped relay connection through the home kernel, and keeps the relay token
 * in memory. The home client remains open as the Room authority for display
 * admission; the scoped worker client supplies only target identity/status.
 * Every operation not mapped to a released public request remains fail-closed.
 */
export async function createManagedBrowserComputerParityTransport({ evidenceRoot, signal } = {}) {
  if (typeof evidenceRoot !== "string" || evidenceRoot.trim() === "") {
    throw new Error("managed parity transport requires an external evidenceRoot")
  }

  const homeKernelUrl = requiredOperatorValue(OPERATOR_ENDPOINT_ENV)
  const targetKernelRef = requiredOperatorValue(TARGET_KERNEL_ENV)
  const targetMachineRef = requiredOperatorValue(TARGET_MACHINE_ENV)
  const clientId = requiredOperatorValue(OPERATOR_CLIENT_ENV)
  const sessionId = requiredOperatorValue(OPERATOR_SESSION_ENV)
  requireStandardLocalAuthConfiguration()

  const [{ LocalIpcClient }, requestApi, displayApi] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../../../packages/kernel-client/dist/display-stream.js"),
  ])
  const homeClient = new LocalIpcClient(homeKernelUrl)
  let connection
  try {
    const response = await sendWithAbortSignal(
      homeClient,
      requestApi.resolveKernelClientConnectionRequest({
        kernelRef: targetKernelRef,
        machineRef: targetMachineRef,
        clientId,
        sessionId,
      }),
      signal,
      "resolve target",
    )
    connection = resolvedConnection(response)
  } catch (error) {
    await homeClient.close().catch(() => {})
    throw error
  }

  requireText(connection.relay_url, "KernelClientConnectionResolved.connection.relay_url")
  requireText(connection.relay_token, "KernelClientConnectionResolved.connection.relay_token")
  if (!connection.target_daemon_id && !connection.target_daemon_alias) {
    throw new Error("managed parity target did not return a daemon identity")
  }
  if (connection.target_daemon_id !== undefined && connection.target_daemon_id !== null) {
    requireText(connection.target_daemon_id, "KernelClientConnectionResolved.connection.target_daemon_id")
  }
  if (connection.target_daemon_alias !== undefined && connection.target_daemon_alias !== null) {
    requireText(connection.target_daemon_alias, "KernelClientConnectionResolved.connection.target_daemon_alias")
  }

  let workerClient
  try {
    workerClient = new LocalIpcClient(connection.relay_url, {
      relayAuthToken: connection.relay_token,
      targetDaemonId: connection.target_daemon_id ?? undefined,
      targetDaemonAlias: connection.target_daemon_alias ?? undefined,
    })
    const webSocketModule = createRequire(fileURLToPath(new URL(
      "../../../../packages/kernel-client/dist/ipc.js",
      import.meta.url,
    )))("ws")
    const webSocket = webSocketModule.WebSocket ?? webSocketModule.default ?? webSocketModule
    return {
      ...createManagedBrowserComputerParityTransportFromPublicClient({
        client: workerClient,
        displayClient: homeClient,
        identityClient: workerClient,
        requestApi,
        displayTransport: {
          openSelkiesDisplayStream: displayApi.openSelkiesDisplayStream,
          webSocket,
        },
      }),
      close: async () => {
        await workerClient.close().catch(() => {})
        await homeClient.close().catch(() => {})
      },
    }
  } catch (error) {
    await workerClient?.close().catch(() => {})
    await homeClient.close().catch(() => {})
    throw error
  }
}

/**
 * Compose the transport around the public client's request/response seam.
 * Tests use an externally controlled authenticated endpoint here; production
 * construction above supplies home/worker LocalIpcClient instances, the
 * released request module, and the released display-stream module.
 */
export function createManagedBrowserComputerParityTransportFromPublicClient({
  client,
  displayClient = client,
  identityClient = client,
  requestApi,
  displayTransport,
} = {}) {
  if (!client || typeof client.send !== "function") {
    throw new Error("managed parity transport requires a public kernel client")
  }
  if (!displayClient || typeof displayClient.send !== "function") {
    throw new Error("managed parity transport requires a public display client")
  }
  if (!identityClient || typeof identityClient.send !== "function") {
    throw new Error("managed parity transport requires a public identity client")
  }
  if (!requestApi || typeof requestApi.getSliceDisplayEndpointRequest !== "function") {
    throw new Error("managed parity transport requires released kernel request constructors")
  }
  if (displayTransport !== undefined && typeof displayTransport.openSelkiesDisplayStream !== "function") {
    throw new Error("managed parity transport requires a released Selkies display-stream opener")
  }
  if (displayTransport !== undefined && (typeof requestApi.relayStatusRequest !== "function"
    || typeof requestApi.getRoomEnvironmentStateRequest !== "function")) {
    throw new Error("managed parity transport requires released identity and Room request constructors")
  }

  return {
    async run(step, request, { signal } = {}) {
      if (step === "selkies.providers") {
        if (signal?.aborted) {
          throw new Error("managed parity selkies.providers was aborted before the public request")
        }
        return runSelkiesProviderAcceptance({
          homeClient: displayClient,
          workerClient: client,
          requestApi,
          request,
          signal,
        })
      }
      if (step !== "selkies.attach") {
        throw new Error(`unsupported managed parity step: ${step}`)
      }
      if (signal?.aborted) {
        throw new Error("managed parity selkies.attach was aborted before the public request")
      }
      return runSelkiesAttach({
        client,
        displayClient,
        identityClient,
        requestApi,
        displayTransport,
        request,
        signal,
      })
    },
  }
}

async function runSelkiesAttach({
  client,
  displayClient,
  identityClient,
  requestApi,
  displayTransport,
  request,
  signal,
}) {
  const binding = request?.binding
  if (!binding || typeof binding !== "object") {
    throw new Error("managed parity selkies.attach requires a target binding")
  }
  if (request.displayBackend !== "selkies") {
    throw new Error("managed parity selkies.attach requires the selkies display backend")
  }
  if (!["web", "local_tui", "remote_tui"].includes(request.client)) {
    throw new Error("managed parity selkies.attach requires a documented client")
  }

  if (displayTransport?.openSelkiesDisplayStream) {
    if (!hasText(request.sliceId) || !hasText(request.attachmentId)) {
      throw new Error(
        "managed parity selkies.attach requires sliceId and attachmentId for a public display stream",
      )
    }
    const roomId = requireText(binding.roomId, "binding.roomId")
    const identity = await readAuthoritativeBinding({
      displayClient,
      identityClient,
      requestApi,
      roomId,
      signal,
    })
    assertBinding(identity, binding, "selkies.attach")
    let stream
    try {
      stream = await displayTransport.openSelkiesDisplayStream({
        client: displayClient,
        sliceId: request.sliceId.trim(),
        sessionId: roomId,
        attachmentId: request.attachmentId.trim(),
        webSocket: displayTransport.webSocket,
        signal,
        connectTimeoutMs: request.connectTimeoutMs ?? DISPLAY_CONNECT_TIMEOUT_MS,
      })
      await stream.sendControl("START_VIDEO", { signal })
      const readyDeadline = Date.now() + (request.streamReadyTimeoutMs ?? DISPLAY_READY_TIMEOUT_MS)
      const startupMessage = await receiveDisplayMessageUntil(
        stream,
        signal,
        readyDeadline,
        (message) => message.kind === "text" && new TextDecoder().decode(message.data) === "VIDEO_STARTED",
        "VIDEO_STARTED",
      )
      const firstFrame = await receiveDisplayMessageUntil(
        stream,
        signal,
        readyDeadline,
        (message) => message.kind === "binary"
          && message.data.byteLength > 10
          && message.data.byteLength <= 4 * 1024 * 1024
          && message.data[0] === 4,
        "a valid Selkies video frame",
      )
      return {
        ...identity,
        client: request.client,
        displayBackend: request.displayBackend,
        attached: true,
        displayProtocol: stream.endpoint.stream_protocol,
        displayStreamId: stream.endpoint.stream_id,
        startupMessage: {
          kind: startupMessage.kind,
          byteLength: startupMessage.data.byteLength,
        },
        firstFrame: {
          kind: firstFrame.kind,
          byteLength: firstFrame.data.byteLength,
          recordType: firstFrame.data[0],
        },
      }
    } finally {
      await stream?.close()
    }
  }

  if (!hasText(request.sliceId) || !hasText(request.attachmentId) || !hasText(request.viewerPublicKey)) {
    throw new Error(
      "managed parity selkies.attach requires sliceId, attachmentId, and viewerPublicKey for public display authorization",
    )
  }
  const sliceId = request.sliceId.trim()
  const attachmentId = request.attachmentId.trim()
  const viewerPublicKey = request.viewerPublicKey.trim()
  const displayEndpointResponse = await sendWithAbortSignal(
    client,
    requestApi.getSliceDisplayEndpointRequest(sliceId, {
      sessionId: requireText(binding.roomId, "binding.roomId"),
      attachmentId,
      viewerPublicKey,
    }),
    signal,
    "selkies.attach display authorization",
  )
  const endpoint = responseVariant(
    displayEndpointResponse,
    "SliceDisplayEndpoint",
    "selkies.attach display authorization",
  ).endpoint
  if (!endpoint || typeof endpoint !== "object") {
    throw new Error("managed parity selkies.attach display authorization returned no endpoint")
  }
  if (endpoint.slice_id !== sliceId || endpoint.kind !== "selkies") {
    throw new Error("managed parity selkies.attach display authorization returned the wrong Selkies endpoint")
  }
  requireText(endpoint.url, "SliceDisplayEndpoint.endpoint.url")
  requireText(endpoint.access, "SliceDisplayEndpoint.endpoint.access")
  requireText(endpoint.stream_protocol, "SliceDisplayEndpoint.endpoint.stream_protocol")
  requireText(endpoint.stream_id, "SliceDisplayEndpoint.endpoint.stream_id")
  requireText(endpoint.peer_public_key, "SliceDisplayEndpoint.endpoint.peer_public_key")
  throw new Error(
    "managed parity selkies.attach has no public Selkies display-stream connection API after authorization",
  )
}

async function receiveDisplayMessageUntil(stream, signal, deadline, matches, description) {
  while (true) {
    const remainingMs = deadline - Date.now()
    if (remainingMs <= 0) {
      throw new Error(`managed parity selkies.attach did not receive ${description} before the deadline`)
    }
    const message = await stream.receive({ signal, timeoutMs: remainingMs })
    if (matches(message)) return message
  }
}

async function readAuthoritativeBinding({ displayClient, identityClient, requestApi, roomId, signal }) {
  const relayResponse = await sendWithAbortSignal(
    identityClient,
    requestApi.relayStatusRequest(),
    signal,
    "selkies.attach relay identity",
  )
  const relayStatus = responseVariant(relayResponse, "RelayStatus", "selkies.attach relay identity").status
  if (relayStatus?.configured !== true || relayStatus.connected !== true) {
    throw new Error("managed parity selkies.attach requires a connected configured relay")
  }
  const environmentResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.getRoomEnvironmentStateRequest(roomId),
    signal,
    "selkies.attach Room state",
  )
  const environment = responseVariant(
    environmentResponse,
    "RoomEnvironmentState",
    "selkies.attach Room state",
  ).environment
  return {
    kernelId: requireText(relayStatus?.daemon_id, "RelayStatus.status.daemon_id"),
    machineId: requireText(relayStatus?.machine_id, "RelayStatus.status.machine_id"),
    roomId: requireText(environment?.session_id, "RoomEnvironmentState.environment.session_id"),
    environmentId: requireText(environment?.environment_id, "RoomEnvironmentState.environment.environment_id"),
  }
}

function assertBinding(value, binding, step) {
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    if (value[field] !== binding[field]) {
      throw new Error(`managed parity target identity mismatch for ${step}: ${field}`)
    }
  }
}

async function sendWithAbortSignal(client, request, signal, step) {
  if (signal?.aborted) {
    closeAfterAbort(client)
    throw abortError(step)
  }
  const operation = Promise.resolve().then(() => client.send(request))
  if (!signal) return operation

  let abort
  const aborted = new Promise((_, reject) => {
    abort = () => {
      closeAfterAbort(client)
      reject(abortError(step))
    }
  })
  signal.addEventListener("abort", abort, { once: true })
  if (signal.aborted) abort()
  try {
    return await Promise.race([operation, aborted])
  } finally {
    signal.removeEventListener("abort", abort)
  }
}

function abortError(step) {
  const error = new Error(`managed parity ${step} was aborted while the public request was in flight`)
  error.name = "AbortError"
  return error
}

function closeAfterAbort(client) {
  const close = typeof client.close === "function"
    ? client.close
    : typeof client.destroy === "function"
      ? client.destroy
      : null
  if (!close) return
  try {
    Promise.resolve(close.call(client)).catch(() => {})
  } catch {
    // Abort must reject the public operation even if transport teardown fails.
  }
}

function resolvedConnection(response) {
  const connection = responseVariant(response, "KernelClientConnectionResolved", "resolve target").connection
  if (!connection || typeof connection !== "object") {
    throw new Error("managed parity target connection response was malformed")
  }
  return connection
}

function responseVariant(response, variant, step) {
  if (!response || typeof response !== "object" || !(variant in response)) {
    throw new Error(`managed parity ${step} returned an unexpected response; expected ${variant}`)
  }
  return response[variant]
}

function requiredOperatorValue(name) {
  const value = process.env[name]?.trim()
  if (!value) throw new Error(`${name} is required`)
  return value
}

function requireStandardLocalAuthConfiguration() {
  if (process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN === undefined
    && process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE === undefined) {
    throw new Error(
      "managed parity requires CHARIOX_KERNEL_LOCAL_AUTH_TOKEN or CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    )
  }
}

function requireText(value, label) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`managed parity response requires ${label}`)
  }
  return value
}

function hasText(value) {
  return typeof value === "string" && value.trim() !== ""
}
