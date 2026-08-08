import { randomUUID } from "node:crypto"

import type { WorkflowDesignOp } from "@arroba/kernel-client/kernel-types"

import type { RuntimeSession } from "./cli-types.js"
import { applyWorkflowDesignOpRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowDesignOpControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  originClientId: string
}

export function createWorkflowDesignOpController(deps: WorkflowDesignOpControllerDeps) {
  const applyWorkflowDesignOp = async (op: WorkflowDesignOp) => {
    const response = await deps.sendRequest(applyWorkflowDesignOpRequest(
      deps.sessionId(),
      deps.originClientId,
      `tui-${randomUUID()}`,
      op,
    ))
    return expectVariant<{ session: RuntimeSession }>(response, "WorkflowDesignOpAccepted")
  }

  const createWorkflowDesignId = (prefix: string) => (
    `${prefix}-${randomUUID().replace(/-/g, "").slice(0, 16)}`
  )

  return { applyWorkflowDesignOp, createWorkflowDesignId }
}
