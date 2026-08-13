import process from "node:process"

import type {
  RuntimeNoticeRecord,
  RuntimeSession,
  TerminalOutputRecord,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"
import {
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
  resizeTerminalRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import { describeCliError } from "./runtime.js"
import { sessionNeedsAttachedRuntimeCatchUp } from "@chariox/kernel-client/provider-run-recovery"

export async function pumpTerminalOutput(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
): Promise<TerminalOutputRecord[]> {
  const response = await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
  const payload = expectVariant<{ records: TerminalOutputRecord[] }>(response, "TerminalOutput")
  return payload.records
}

export async function pollRuntimeNotices(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
): Promise<RuntimeNoticeRecord[]> {
  const response = await client.send<Record<string, unknown>>(pollRuntimeNoticesRequest(sessionId, attachmentId))
  const payload = expectVariant<{ notices: RuntimeNoticeRecord[] }>(response, "RuntimeNotices")
  return payload.notices
}

export async function catchUpAttachedSession(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  session: RuntimeSession,
  logger?: CharioxLogger | null,
): Promise<void> {
  if (!sessionNeedsAttachedRuntimeCatchUp(session)) {
    return
  }

  try {
    await pumpTerminalOutput(client, sessionId, attachmentId)
    await pollRuntimeNotices(client, sessionId, attachmentId)
  } catch (error) {
    logger?.warn("attached session catch-up failed", {
      session_id: sessionId,
      attachment_id: attachmentId,
      error: describeCliError(error),
    })
  }
}

export async function resizeSessionTerminal(client: LocalIpcClient, sessionId: string): Promise<void> {
  if (!process.stdout.isTTY || !process.stdout.columns || !process.stdout.rows) {
    return
  }
  try {
    await client.send<Record<string, unknown>>(resizeTerminalRequest(sessionId, process.stdout.columns, process.stdout.rows))
  } catch {
    // Terminal sizing is advisory; runtime polling owns connection recovery.
  }
}
