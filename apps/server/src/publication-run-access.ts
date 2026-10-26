// MP-11: trace exposure selects fields only after caller ownership is established.
import type { VerifiedPublicationCallerClaims } from "./publication-caller-claims.js"
import { resolveWorkflowRequest } from "@chariox/kernel-client/ipc-requests"
import type { KernelLookupClient, WorkflowPublicationConfig, WorkflowRun } from "./publication-types.js"

type PublicationRunScope = Pick<WorkflowPublicationConfig, "publication_id" | "workflow_ref" | "endpoint_ref"> & {
  invocationEndpointRef?: string
}

// Resolve through the kernel when recorded canonical IDs differ from config refs.
// Do not infer a workflow identity from a candidate run or cache across sessions.
export async function resolvePublicationRunScope(
  client: KernelLookupClient,
  publication: WorkflowPublicationConfig,
  runs: readonly WorkflowRun[],
): Promise<PublicationRunScope> {
  if (runs.every((run) => (!run.workflow_id || run.workflow_id === publication.workflow_ref)
    && (!run.endpoint_id || run.endpoint_id === publication.endpoint_ref)
    && (!run.publication_invocation || run.publication_invocation.endpoint_id === publication.endpoint_ref))) return publication
  const response = await client.send(resolveWorkflowRequest(publication.session_id, publication.workflow_ref))
  const workflow = (response.WorkflowResolved as { workflow?: {
    id: string; endpoints?: Array<{ id: string; alias?: string | null }>
  } } | undefined)?.workflow
  if (!workflow?.id) throw new Error("publication workflow reference could not be resolved")
  const ref = publication.endpoint_ref.trim().toLowerCase()
  const endpoints = workflow.endpoints ?? []
  // Same precedence as the kernel: exact ID, exact alias, unique ID prefix,
  // unique alias prefix. Ambiguous or absent references fail closed.
  const idMatches = endpoints.filter((endpoint) => endpoint.id.startsWith(ref))
  const aliasMatches = endpoints.filter((endpoint) => endpoint.alias?.startsWith(ref))
  const endpoint = endpoints.find((endpoint) => endpoint.id === ref)
    ?? endpoints.find((endpoint) => endpoint.alias === ref)
    ?? (idMatches.length === 1 ? idMatches[0] : undefined)
    ?? (aliasMatches.length === 1 ? aliasMatches[0] : undefined)
  if (!endpoint) throw new Error("publication endpoint reference could not be resolved")
  return { publication_id: publication.publication_id, workflow_ref: workflow.id,
    endpoint_ref: endpoint.id, invocationEndpointRef: publication.endpoint_ref }
}

export function publicationRunAccessibleToCaller(
  publication: PublicationRunScope,
  run: WorkflowRun,
  caller: VerifiedPublicationCallerClaims | null,
): boolean {
  if (run.workflow_id && run.workflow_id !== publication.workflow_ref) return false
  if (run.endpoint_id && run.endpoint_id !== publication.endpoint_ref) return false
  const invocation = run.publication_invocation
  if (invocation && (invocation.publication_id !== publication.publication_id
    || (invocation.endpoint_id !== publication.endpoint_ref
      && invocation.endpoint_id !== publication.invocationEndpointRef))) return false
  // Unclaimed local/public publications retain shared anonymous and legacy runs.
  // Authenticated records are private even if this runtime has no claims verifier.
  if (!caller) return !invocation || invocation.caller?.type === "anonymous"
  if (invocation?.caller?.type !== "authenticated") return false
  const proof = record(record(invocation.caller.proof)?.publication_caller)
  return proof?.subject === caller.subject
    && proof.account_id === caller.accountId
    && proof.deployment_id === caller.deploymentId
    && proof.environment_id === caller.environmentId
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
