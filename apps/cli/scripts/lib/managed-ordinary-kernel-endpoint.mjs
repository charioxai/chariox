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
