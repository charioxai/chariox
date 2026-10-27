import { randomUUID } from "node:crypto"

import type { RuntimeSession, WorkflowDesignOp } from "./kernel-types.js"
import { applyWorkflowDesignOpRequest } from "./ipc-requests.js"

type ShellWorkflowDesignOpDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
  clientId: string
}

export async function applyShellWorkflowDesignOp(
  deps: ShellWorkflowDesignOpDeps,
  sessionId: string,
  op: WorkflowDesignOp,
): Promise<{ event: { op?: unknown }; session: RuntimeSession }> {
  const response = await deps.client.send(applyWorkflowDesignOpRequest(
    sessionId,
    deps.clientId,
    `shell-${randomUUID()}`,
    op,
  ))
  return expectVariant<{ event: { op?: unknown }; session: RuntimeSession }>(
    response,
    "WorkflowDesignOpAccepted",
  )
}

export function createShellWorkflowDesignId(prefix: string): string {
  return `${prefix}-${randomUUID().replace(/-/g, "").slice(0, 16)}`
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
