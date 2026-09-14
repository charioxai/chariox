const OPERATOR_ENDPOINT_ENV = "CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL"
const TARGET_KERNEL_ENV = "CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF"
const TARGET_MACHINE_ENV = "CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF"
const OPERATOR_CLIENT_ENV = "CHARIOX_MANAGED_PARITY_CLIENT_ID"
const OPERATOR_SESSION_ENV = "CHARIOX_MANAGED_PARITY_SESSION_ID"

/**
 * Build the reviewed parity transport from the released public kernel client.
 *
 * The operator supplies only non-secret target references here. LocalIpcClient
 * consumes the normal one-shot local-auth configuration itself, resolves a
 * scoped relay connection through the home kernel, and keeps the relay token
 * in memory. This adapter deliberately implements only the first verified
 * product step; every other harness step fails closed until its real public
 * operation has been mapped.
 */
export async function createManagedBrowserComputerParityTransport({ evidenceRoot } = {}) {
  if (typeof evidenceRoot !== "string" || evidenceRoot.trim() === "") {
    throw new Error("managed parity transport requires an external evidenceRoot")
  }

  const homeKernelUrl = requiredOperatorValue(OPERATOR_ENDPOINT_ENV)
  const targetKernelRef = requiredOperatorValue(TARGET_KERNEL_ENV)
  const targetMachineRef = requiredOperatorValue(TARGET_MACHINE_ENV)
  const clientId = requiredOperatorValue(OPERATOR_CLIENT_ENV)
  const sessionId = requiredOperatorValue(OPERATOR_SESSION_ENV)
  requireStandardLocalAuthConfiguration()

  const [{ LocalIpcClient }, requestApi] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
  ])
  const homeClient = new LocalIpcClient(homeKernelUrl)
  let connection
  try {
    const response = await homeClient.send(requestApi.resolveKernelClientConnectionRequest({
      kernelRef: targetKernelRef,
      machineRef: targetMachineRef,
      clientId,
      sessionId,
    }))
    connection = resolvedConnection(response)
  } finally {
    await homeClient.close().catch(() => {})
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

  const workerClient = new LocalIpcClient(connection.relay_url, {
    relayAuthToken: connection.relay_token,
    targetDaemonId: connection.target_daemon_id ?? undefined,
    targetDaemonAlias: connection.target_daemon_alias ?? undefined,
  })
  return {
    ...createManagedBrowserComputerParityTransportFromPublicClient({
      client: workerClient,
      requestApi,
    }),
    close: () => workerClient.close(),
  }
}

/**
 * Compose the transport around the public client's request/response seam.
 * Tests use an externally controlled authenticated endpoint here; production
 * construction above supplies LocalIpcClient and the released request module.
 */
export function createManagedBrowserComputerParityTransportFromPublicClient({ client, requestApi } = {}) {
  if (!client || typeof client.send !== "function") {
    throw new Error("managed parity transport requires a public kernel client")
  }
  if (!requestApi
    || typeof requestApi.relayStatusRequest !== "function"
    || typeof requestApi.getRoomEnvironmentStateRequest !== "function") {
    throw new Error("managed parity transport requires released kernel request constructors")
  }

  return {
    async run(step, request, { signal } = {}) {
      if (step !== "selkies.attach") {
        throw new Error(`unsupported managed parity step: ${step}`)
      }
      if (signal?.aborted) {
        throw new Error("managed parity selkies.attach was aborted before the public request")
      }
      return runSelkiesAttach({ client, requestApi, request })
    },
  }
}

async function runSelkiesAttach({ client, requestApi, request }) {
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

  const relayResponse = await client.send(requestApi.relayStatusRequest())
  const relayStatus = responseVariant(relayResponse, "RelayStatus", "selkies.attach").status
  if (relayStatus?.configured !== true
    || relayStatus.connected !== true
    || relayStatus.relay_token_configured !== true) {
    throw new Error("managed parity selkies.attach requires a connected configured relay")
  }
  const environmentResponse = await client.send(
    requestApi.getRoomEnvironmentStateRequest(requireText(binding.roomId, "binding.roomId")),
  )
  const environment = responseVariant(
    environmentResponse,
    "RoomEnvironmentState",
    "selkies.attach",
  ).environment
  const result = {
    kernelId: requireText(relayStatus?.daemon_id, "RelayStatus.status.daemon_id"),
    machineId: requireText(relayStatus?.machine_id, "RelayStatus.status.machine_id"),
    roomId: requireText(environment?.session_id, "RoomEnvironmentState.environment.session_id"),
    environmentId: requireText(environment?.environment_id, "RoomEnvironmentState.environment.environment_id"),
    client: request.client,
    displayBackend: request.displayBackend,
  }
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    if (result[field] !== binding[field]) {
      throw new Error(`managed parity target identity mismatch for selkies.attach: ${field}`)
    }
  }
  return result
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
