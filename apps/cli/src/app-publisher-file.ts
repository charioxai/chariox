import { randomUUID } from "node:crypto"
import { setTimeout as delay } from "node:timers/promises"
import { beginAppPublisherEnrollmentRequest, getAppPublisherEnrollmentRequest,
  cancelAppPublisherEnrollmentRequest } from "@chariox/kernel-client/ipc-requests"
import type { AppPublisherEnrollmentSummary } from "@chariox/kernel-client/kernel-types"
import { terminalCwd } from "./app-install-file.js"
import { readPublisherFile } from "./app-publisher-file/source.js"

type Send = (request: Record<string, unknown>) => Promise<Record<string, unknown>>
type Attempt = { identity: string; requestId: string; terminal: boolean; cancelled: boolean }
const finalPhases = ["approved", "denied", "cancelled", "failed"]

/** Retains exact retry identity; every trust decision remains in the kernel. */
export class AppPublisherEnrollment {
  private attempt?: Attempt
  private enrolling: Promise<AppPublisherEnrollmentSummary> | undefined
  private requests = new Set<Promise<unknown>>()
  private disposed = false
  constructor(private send: Send, private notice: (message: string) => void,
    private cwd: string = terminalCwd()) {}

  enroll(selected: string, sessionId: string, expectedRevision = "0"): Promise<AppPublisherEnrollmentSummary> {
    if (this.enrolling) return Promise.reject(new Error("A publisher enrollment request is already running"))
    const job = this.own(async () => {
      if (!sessionId || Buffer.byteLength(sessionId) > 128) throw new Error("Attach to a session before enrolling a publisher")
      if (!/^(0|[1-9][0-9]{0,18})$/.test(expectedRevision) || BigInt(expectedRevision) > 9223372036854775807n) throw new Error("Invalid publisher revision")
      const material = await readPublisherFile(selected, this.cwd)
      if (this.disposed) throw new Error("This terminal is closing")
      const identity = JSON.stringify({ sessionId, expectedRevision, ...material })
      let attempt = this.attempt
      if (!attempt || attempt.identity !== identity || attempt.terminal || attempt.cancelled) {
        attempt = { identity, requestId: `app-publisher-${randomUUID()}`, terminal: false, cancelled: false }
        this.attempt = attempt
      }
      // Print before dispatch, so a lost reply or process exit still leaves a
      // queryable identity. Public-key bytes and local paths are not printed.
      this.notice(`Publisher review ${attempt.requestId}. Use /app publisher status ${attempt.requestId} or /app publisher cancel ${attempt.requestId}.`)
      return this.request(beginAppPublisherEnrollmentRequest({ sessionId, expectedRevision,
        requestId: attempt.requestId, ...material }), attempt.requestId, () => attempt.cancelled)
    })
    this.enrolling = job
    void job.finally(() => { if (this.enrolling === job) this.enrolling = undefined }).catch(() => {})
    return job
  }

  status(requestId?: string): Promise<AppPublisherEnrollmentSummary> {
    return this.own(() => {
      const id = this.id(requestId)
      return this.request(getAppPublisherEnrollmentRequest(id), id)
    })
  }

  cancel(requestId?: string): Promise<AppPublisherEnrollmentSummary> {
    return this.own(async () => {
      // A still-reading enrollment cannot have reached the kernel yet. Wait
      // for its retained request to settle before choosing the operation ID.
      const enrolling = this.enrolling
      if (this.attempt && (!requestId || requestId === this.attempt.requestId)) this.attempt.cancelled = true
      await enrolling?.catch(() => {})
      const id = this.id(requestId)
      if (this.attempt?.requestId === id) this.attempt.cancelled = true
      // NotFound is not confirmation of cancellation after a lost Begin reply.
      return this.request(cancelAppPublisherEnrollmentRequest(id), id)
    })
  }

  async dispose(): Promise<void> {
    this.disposed = true
    await Promise.allSettled([...this.requests])
  }

  private id(selected?: string): string {
    const id = selected ?? this.attempt?.requestId
    if (!id || id.length > 128 || /[\s\x00-\x1f\x7f]/.test(id)) throw new Error("Supply a publisher review ID, or enroll a public publisher file first")
    return id
  }
  private own<T>(run: () => Promise<T>): Promise<T> {
    if (this.disposed) return Promise.reject(new Error("This terminal is closing"))
    if (this.requests.size >= 2) return Promise.reject(new Error("A publisher status or cancellation request is already pending"))
    const job = run()
    this.requests.add(job)
    void job.finally(() => this.requests.delete(job)).catch(() => {})
    return job
  }
  private async request(request: Record<string, unknown>, id: string, cancelled = () => false): Promise<AppPublisherEnrollmentSummary> {
    for (let retry = 0; retry < 3; retry++) {
      if (cancelled()) throw new Error(`Publisher review ${id} is being cancelled`)
      let reply: Record<string, unknown>
      try { reply = await this.send(request) }
      catch {
        if (retry === 2) throw new Error(`Connection interrupted; publisher review ${id} is not confirmed. Check /app publisher status ${id}.`)
        await delay(200); continue
      }
      const failure = reply.AppRequestFailed as { code?: unknown } | undefined
      if (failure) {
        if (failure.code === "busy" && retry < 2) { await delay(200); continue }
        const messages: Record<string, string> = { unauthorized: "This connection cannot review publishers", not_found: "Publisher review is not yet found; no cancellation or enrollment is confirmed",
          conflict: "The publisher review changed; inspect its status", limit_exceeded: "The kernel's publisher review limit has been reached", storage_unavailable: "Publisher storage is unavailable", invalid_request: "The kernel rejected the public publisher material" }
        throw new Error(`${messages[String(failure.code)] ?? "Publisher request failed"}. Review ${id}.`)
      }
      const value = (reply.AppPublisherEnrollmentStatus as { operation?: AppPublisherEnrollmentSummary } | undefined)?.operation
      if (!value || value.request_id !== id || !["pending", ...finalPhases].includes(value.phase)) throw new Error("Kernel returned an invalid publisher review receipt")
      if (this.attempt?.requestId === id) this.attempt.terminal = finalPhases.includes(value.phase)
      return value
    }
    throw new Error("Publisher request could not complete")
  }
}

export function formatPublisherReview(value: AppPublisherEnrollmentSummary): string {
  const label = { pending: "Awaiting publisher approval in the review session", approved: "Publisher approval recorded",
    denied: "Publisher enrollment declined", cancelled: "Publisher review cancelled", failed: "Publisher enrollment failed" }[value.phase]
  const revision = value.approved_revision ? ` Revision ${value.approved_revision}; later revocation can supersede this approval.` : ""
  return `${label}: ${value.publisher_id}.${revision} Review ${value.request_id}.`
}
