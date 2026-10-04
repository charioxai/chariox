// MP-08 / MP-10 / MP-11: retain the first RPC failure before salvage or cleanup.
import { sanitizeDrillMetadata } from '../../lib/drill-secrets.mjs'

export function rpcErrorRecord(error) {
  return sanitizeDrillMetadata({
    name: error?.name ?? 'Error',
    operation: error?.operation ?? null,
    code: error?.code ?? null,
    retryable: error?.retryable ?? false,
    message: String(error?.message ?? error).slice(0, 8192),
  })
}

export function observeKernelRpcErrors(client, retain) {
  const send = client.send.bind(client)
  client.send = async request => {
    const startedAt = Date.now()
    try { return await send(request) }
    catch (error) {
      const failure = { requestKind: Object.keys(request ?? {})[0] ?? 'unknown',
        startedAt, elapsedMs: Date.now() - startedAt, error: rpcErrorRecord(error) }
      // Evidence-storage failure must not replace the original transport error.
      try { await retain(failure) } catch { /* The caller still receives the original. */ }
      throw error
    }
  }
}
