// MP-10 transport selection is a locator, never capture authority.
export function managedOrdinaryKernelConnection(environment, legacySocketPath) {
  const configured = environment?.CHARIOX_KERNEL_URL?.trim()
  if (!configured) return { endpoint: legacySocketPath(), transport: "local-unix-ipc", clientOptions: {} }
  let url
  try { url = new URL(configured) } catch { throw new Error("kernel public API endpoint is invalid") }
  if (url.username || url.password || url.search || url.hash) throw new Error("kernel endpoint must not contain credentials or query parameters")
  const relayAuthToken = environment?.CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN?.trim()
  if (relayAuthToken) {
    if (url.protocol !== "wss:") throw new Error("relay capture requires the authenticated TLS product route")
    let capture
    try { capture = JSON.parse(environment.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON ?? "null") } catch {}
    const targetDaemonId = capture?.kernel_identity?.kernel_id
    if (typeof targetDaemonId !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(targetDaemonId)) {
      throw new Error("relay capture requires an exact product target kernel locator")
    }
    return { endpoint: url.href, transport: "relay", clientOptions: { relayAuthToken, targetDaemonId } }
  }
  if (url.protocol !== "ws:" || !["127.0.0.1", "[::1]"].includes(url.hostname)
    || !["/", "/kernel"].includes(url.pathname)) throw new Error("local kernel capture requires an exact loopback product endpoint")
  url.pathname = "/"
  return { endpoint: url.href, transport: "kernel-public-api", clientOptions: {} }
}

// MP-10: LocalIpcClient consumes one-shot credentials. Keep the admitted
// context only for collector-owned probes; never send it to Git/provider
// version commands or retain it in evidence. This creates no kernel credential.
export async function prepareManagedOrdinaryProbeEnvironment(environment = process.env, {
  consumeLocalAuthToken,
} = {}) {
  const probeEnvironment = { ...environment }
  delete probeEnvironment.CHARIOX_PARITY_SIGNING_KEY
  const connection = managedOrdinaryKernelConnection(environment, () => {
    if (environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined) {
      throw new Error("MP-10 local auth requires an explicit loopback kernel endpoint")
    }
    return environment.CHARIOX_DAEMON_SOCKET
  })
  if (connection.transport === "relay") {
    if (environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined) {
      throw new Error("MP-10 relay and loopback credentials cannot be combined")
    }
  } else if (connection.transport === "kernel-public-api") {
    const consume = consumeLocalAuthToken
      ?? (await import(new URL("../../dist/ipc.js", import.meta.url))).consumeKernelLocalAuthTokenFromEnv
    const token = await consume(connection.endpoint)
    delete probeEnvironment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
    delete probeEnvironment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
    if (token) probeEnvironment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN = token
  }
  return probeEnvironment
}
