// MP-11: trace exposure selects fields only after caller ownership is established.
import type { VerifiedPublicationCallerClaims } from "./publication-caller-claims.js"
import type { WorkflowPublicationConfig, WorkflowRun } from "./publication-types.js"

export function publicationRunAccessibleToCaller(
  publication: WorkflowPublicationConfig,
  run: WorkflowRun,
  caller: VerifiedPublicationCallerClaims | null,
): boolean {
  if (run.workflow_id && run.workflow_id !== publication.workflow_ref) return false
  if (run.endpoint_id && run.endpoint_id !== publication.endpoint_ref) return false
  const invocation = run.publication_invocation
  if (invocation && (invocation.publication_id !== publication.publication_id
    || invocation.endpoint_id !== publication.endpoint_ref)) return false
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
