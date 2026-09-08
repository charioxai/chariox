/** Terminal-owned local file upload. Kernel approval and execution stay remote. */
import { randomUUID } from "node:crypto"
import { isAbsolute, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import {
  beginAppPackageUploadRequest, putAppPackageUploadChunkRequest, abortAppPackageUploadRequest,
  beginAppInstallRequest, getAppInstallOperationRequest, cancelAppInstallOperationRequest,
} from "@chariox/kernel-client/ipc-requests"
import type { AppInstallOperationSummary, AppPackageUploadSummary } from "@chariox/kernel-client/kernel-types"
import { AppFileSource, checkCancelled, chunkBytes, InstallCancelled, InstallFileChanged } from "./app-install-file/source.js"

type Send = (request: Record<string, unknown>) => Promise<Record<string, unknown>>
export type InstallProgress = { phase: "hashing" | "uploading"; bytes: number; total: number }
type Attempt = {
  path: string; session: string; uploadRequest: string; request: string; cancelled: boolean;
  digest?: string; size?: number; handle?: string; beginSent: boolean; status?: AppInstallOperationSummary; closed: boolean;
}
class KernelFailure extends Error { constructor(readonly code: string) { super(messages[code] ?? `App request failed: ${code}`) } }
class ConnectionFailure extends Error { constructor() { super("Connection interrupted. Run the same /app install command to resume this attempt, or /app cancel to cancel it.") } }
const messages: Record<string, string> = {
  unauthorized: "This connection is not authorized to install Apps.", busy: "App requests are busy. Try again shortly.",
  conflict: "The App operation changed; check /app operation.", not_found: "App operation or upload was not found.",
  storage_unavailable: "App storage is unavailable. Retry this operation when the kernel is available.",
  limit_exceeded: "The kernel's App storage or operation limit has been reached.",
}

/** One transfer per terminal; reconnect retries retain the exact original IDs. */
export class AppFileInstaller {
  private attempt: Attempt | undefined
  private running: Promise<AppInstallOperationSummary> | undefined
  private cleaning: Promise<AppInstallOperationSummary | undefined> | undefined
  private disposed = false
  private operations = new Set<Promise<unknown>>()
  constructor(private send: Send, private progress: (value: InstallProgress) => void = () => {},
    private cwd: string = terminalCwd()) {}

  install(selected: string, session: string): Promise<AppInstallOperationSummary> {
    if (this.disposed) return Promise.reject(new Error("This terminal is closing"))
    if (this.running) return Promise.reject(new Error("An App upload is already running. Use /app cancel to stop it."))
    if (!session || Buffer.byteLength(session) > 128) return Promise.reject(new Error("Attach to a session before installing an App"))
    const path = resolve(this.cwd, selected)
    let attempt = this.attempt
    if (!attempt || attempt.closed) {
      attempt = { path, session, uploadRequest: `app-upload-${randomUUID()}`, request: `app-install-${randomUUID()}`, cancelled: false, beginSent: false, closed: false }
      this.attempt = attempt
    } else if (attempt.path !== path || attempt.session !== session) {
      return Promise.reject(new Error("Another App installation is retained. Use /app operation or /app cancel before selecting another file."))
    }
    const operation = this.transfer(attempt)
    this.running = operation
    void operation.finally(() => { if (this.running === operation) this.running = undefined }).catch(() => {})
    return operation
  }

  status(requestId?: string): Promise<AppInstallOperationSummary> {
    return this.own(() => this.loadStatus(requestId))
  }

  private async loadStatus(requestId?: string): Promise<AppInstallOperationSummary> {
    const attempt = this.attempt
    const request = requestId ?? attempt?.request
    if (!request) throw new Error("No App installation in this terminal. Use /app operation <request-id> to inspect another attempt.")
    const status = operation(await this.request(getAppInstallOperationRequest(request), () => false), request)
    if (attempt?.request === request) { attempt.status = status; await this.releaseUpload(attempt, status) }
    return status
  }

  cancel(requestId?: string): Promise<AppInstallOperationSummary | undefined> {
    return this.own(() => this.cancelAttempt(requestId))
  }

  private async cancelAttempt(requestId?: string): Promise<AppInstallOperationSummary | undefined> {
    const attempt = this.attempt
    if (requestId && requestId !== attempt?.request) return operation(await this.request(cancelAppInstallOperationRequest(requestId), () => false), requestId)
    if (!attempt) throw new Error("No App installation in this terminal")
    attempt.cancelled = true
    // Never race Abort ahead of an outstanding Begin/PutChunk/BeginInstall.
    // The transfer retains its exact request until the transport settles it.
    await this.running?.catch(() => {})
    return this.cleanup(attempt)
  }

  /** Caller owns this promise through terminal shutdown; begun installs remain kernel-owned. */
  async dispose(): Promise<void> {
    this.disposed = true
    const attempt = this.attempt
    if (!attempt) { await Promise.allSettled([...this.operations]); return }
    if (!attempt.beginSent) attempt.cancelled = true
    await this.running?.catch(() => {})
    await Promise.allSettled([...this.operations])
    await this.cleaning?.catch(() => {})
    if (!attempt.beginSent) await this.cleanup(attempt).catch(() => {})
  }

  private own<T>(run: () => Promise<T>): Promise<T> {
    if (this.disposed) return Promise.reject(new Error("This terminal is closing"))
    if (this.operations.size >= 2) return Promise.reject(new Error("An App status or cancellation request is already pending"))
    const job = run()
    this.operations.add(job)
    void job.finally(() => this.operations.delete(job)).catch(() => {})
    return job
  }

  private async transfer(attempt: Attempt): Promise<AppInstallOperationSummary> {
    const cancelled = () => attempt.cancelled || (this.disposed && !attempt.beginSent)
    let source: AppFileSource | undefined
    try {
      checkCancelled(cancelled)
      if (attempt.beginSent) {
        try { return await this.loadStatus(attempt.request) } catch (error) { if (!(error instanceof KernelFailure) || error.code !== "not_found") throw error }
      }
      source = await AppFileSource.open(attempt.path, this.cwd, cancelled, (bytes, total) => this.progress({ phase: "hashing", bytes, total }))
      if (attempt.digest && (attempt.digest !== source.digest || attempt.size !== source.size)) throw new InstallFileChanged()
      attempt.digest = source.digest; attempt.size = source.size
      const upload = uploadStatus(await this.request(beginAppPackageUploadRequest({ requestId: attempt.uploadRequest, expectedSize: source.size, sha256: source.digest }), cancelled), source)
      attempt.handle = upload.handle
      checkCancelled(cancelled)
      if (upload.phase === "aborted") throw new Error("This upload was cancelled or expired. Cancel the retained attempt before installing again.")
      if (upload.accepted_bytes !== source.size && upload.accepted_bytes % chunkBytes !== 0) throw new Error("Kernel returned an invalid App upload offset")
      for (let offset = upload.accepted_bytes; offset < source.size;) {
        checkCancelled(cancelled)
        const chunk = await source.chunk(offset)
        const next = uploadStatus(await this.request(putAppPackageUploadChunkRequest({ handle: upload.handle, offset, dataBase64: chunk.bytes.toString("base64"), chunkSha256: chunk.sha256 }), cancelled), source)
        if (next.handle !== upload.handle || next.accepted_bytes !== offset + chunk.bytes.length || next.phase !== "receiving") throw new Error("Kernel returned an inconsistent App upload receipt")
        offset = next.accepted_bytes
        this.progress({ phase: "uploading", bytes: offset, total: source.size })
      }
      await source.unchanged()
      checkCancelled(cancelled)
      attempt.beginSent = true
      const status = operation(await this.request(beginAppInstallRequest({ sessionId: attempt.session, requestId: attempt.request, uploadHandle: upload.handle, expectedPackageDigest: source.digest }), cancelled), attempt.request)
      attempt.status = status
      checkCancelled(cancelled)
      await this.releaseUpload(attempt, status)
      return status
    } catch (error) {
      if (error instanceof InstallCancelled || error instanceof InstallFileChanged) await this.cleanup(attempt).catch(() => {})
      throw error
    } finally { await source?.close() }
  }

  private cleanup(attempt: Attempt): Promise<AppInstallOperationSummary | undefined> {
    if (this.cleaning) return this.cleaning
    const clean = (async () => {
      let status = attempt.status
      if (attempt.beginSent) {
        try { status = operation(await this.request(cancelAppInstallOperationRequest(attempt.request), () => false), attempt.request) }
        catch (error) {
          if (error instanceof KernelFailure && error.code === "conflict") status = await this.loadStatus(attempt.request)
          else if (!(error instanceof KernelFailure) || error.code !== "not_found") throw error
        }
      }
      if (status?.phase === "preparing" || status?.phase === "starting" || status?.phase === "awaiting_approval") return status
      if (!attempt.handle && attempt.digest && attempt.size) {
        const reply = await this.request(beginAppPackageUploadRequest({ requestId: attempt.uploadRequest, expectedSize: attempt.size, sha256: attempt.digest }), () => false)
        const upload = reply.AppPackageUploadStatus as { upload?: { handle?: unknown } } | undefined
        if (typeof upload?.upload?.handle !== "string" || !/^upload_[0-9a-f]{64}$/.test(upload.upload.handle)) throw new Error("Kernel returned an invalid upload receipt")
        attempt.handle = upload.upload.handle
      }
      if (attempt.handle) await this.request(abortAppPackageUploadRequest(attempt.handle), () => false)
      attempt.closed = true
      if (status) attempt.status = status
      return status
    })()
    this.cleaning = clean
    void clean.finally(() => { if (this.cleaning === clean) this.cleaning = undefined }).catch(() => {})
    return clean
  }

  private async releaseUpload(attempt: Attempt, status: AppInstallOperationSummary): Promise<void> {
    if (status.phase === "preparing") return
    if (attempt.handle) await this.request(abortAppPackageUploadRequest(attempt.handle), () => false).catch(() => {})
    if (["committed", "cancelled", "failed"].includes(status.phase)) attempt.closed = true
  }

  private async request(request: Record<string, unknown>, cancelled: () => boolean): Promise<Record<string, unknown>> {
    for (let retry = 0; retry < 3; retry++) {
      checkCancelled(cancelled)
      try {
        const response = await this.send(request)
        const failure = response.AppRequestFailed as { code?: unknown } | undefined
        if (!failure) return response
        const code = typeof failure.code === "string" ? failure.code : "invalid_response"
        if (code !== "busy" || retry === 2) throw new KernelFailure(code)
      } catch (error) {
        if (error instanceof KernelFailure || error instanceof InstallCancelled) throw error
        if (retry === 2) throw new ConnectionFailure()
      }
      await delay(200)
    }
    throw new ConnectionFailure()
  }
}

function uploadStatus(reply: Record<string, unknown>, source: AppFileSource): AppPackageUploadSummary {
  const value = (reply.AppPackageUploadStatus as { upload?: AppPackageUploadSummary } | undefined)?.upload
  if (!value || !/^upload_[0-9a-f]{64}$/.test(value.handle) || value.expected_size !== source.size || value.sha256 !== source.digest || !Number.isSafeInteger(value.accepted_bytes) || value.accepted_bytes < 0 || value.accepted_bytes > source.size || !["receiving", "finalized", "aborted"].includes(value.phase)) throw new Error("Kernel returned an invalid App upload receipt")
  return value
}
function operation(reply: Record<string, unknown>, request: string): AppInstallOperationSummary {
  const value = (reply.AppInstallOperationStatus as { operation?: AppInstallOperationSummary } | undefined)?.operation
  if (!value || value.request_id !== request || !["preparing", "awaiting_approval", "starting", "committed", "cancelled", "failed"].includes(value.phase)) throw new Error("Kernel returned an invalid App installation receipt")
  return value
}

export function formatInstallOperation(value: AppInstallOperationSummary): string {
  const label = { preparing: "Preparing App", awaiting_approval: "Awaiting approval in the installation session", starting: "Starting App", committed: "App installed", cancelled: "App installation cancelled", failed: "App installation failed" }[value.phase]
  const failures: Record<string, string> = {
    app_install_publisher_not_enrolled: "This publisher must be enrolled in the kernel before installation.",
    app_install_publisher_revoked: "This publisher has been revoked in the kernel.",
    app_install_package_rejected: "The kernel rejected the package signature or contents.",
    app_install_upload_missing_or_expired: "The upload expired before preparation. Select the file again.",
    app_install_approval_expired: "The approval request expired.",
    app_install_insufficient_storage: "The kernel has insufficient App storage.",
  }
  const fallback = value.failure && /^app_install_[a-z_]{1,96}$/.test(value.failure) ? `Kernel failure: ${value.failure}.` : "The kernel could not complete installation."
  const detail = value.failure ? ` ${failures[value.failure] ?? fallback}` : ""
  return `${label}${value.installation_id ? `: ${value.installation_id}` : ""}.${detail} Operation ${value.request_id}. Use /app operation for status; /app cancel to cancel before installation completes.`
}

export function formatInstallProgress(value: InstallProgress): string {
  return `${value.phase === "hashing" ? "Checking App file" : "Uploading App"}: ${Math.floor(value.bytes * 100 / value.total)}%`
}
function terminalCwd(): string {
  const captured = process.env.CHARIOX_CLI_ORIGINAL_CWD
  return captured && isAbsolute(captured) ? captured : process.cwd()
}
