// MP-08 / MP-10 / MP-11: optional lifecycle diagnostics omit bodies and credentials.
import { appendFileSync } from "node:fs"
import { createHash } from "node:crypto"

type TraceFields = { method?: string, promptId?: string | null, turnId?: string | null,
  threadId?: string | null, clientId?: string | null, activePromptId?: string | null,
  knownPromptIds?: string[], activeCount?: number, settlement?: string,
  hasClientUserMessageId?: boolean, hasClientId?: boolean, responseKinds?: string[],
  records?: number, outputDrained?: boolean, errorClass?: string, errorCode?: string, reasonClass?: string,
  decodedBytes?: number, compressedBytes?: number, fragmentOffset?: number }
type Trace = { [K in keyof TraceFields]?: TraceFields[K] | undefined }
const hash = (value: string | null | undefined) => typeof value === "string"
  ? createHash("sha256").update(value).digest("hex") : null
export function codexLifecycleTraceRecord(stage: string, data: Trace) {
  return { stage, at: Date.now(), method: data.method, prompt: hash(data.promptId),
    turn: hash(data.turnId), thread: hash(data.threadId), client: hash(data.clientId),
    activePrompt: hash(data.activePromptId), knownPrompts: data.knownPromptIds?.map(hash),
    activeCount: data.activeCount, settlement: data.settlement,
    hasClientUserMessageId: data.hasClientUserMessageId, hasClientId: data.hasClientId,
    responseKinds: data.responseKinds, records: data.records, outputDrained: data.outputDrained,
    errorClass: data.errorClass, errorCode: data.errorCode, reasonClass: data.reasonClass,
    decodedBytes: publicByteCount(data.decodedBytes), compressedBytes: publicByteCount(data.compressedBytes),
    fragmentOffset: publicByteCount(data.fragmentOffset) }
}
function publicByteCount(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= 17 * 1024 * 1024 ? value : undefined
}
export function traceCodexLifecycle(stage: string, data: Trace) {
  const path = process.env.CHARIOX_CODEX_LIFECYCLE_TRACE_FILE
  if (!path) return
  try { appendFileSync(path, JSON.stringify(codexLifecycleTraceRecord(stage, data)) + "\n", { mode: 0o600 }) }
  catch { /* Diagnostics must not alter the native conversation. */ }
}
