import { createHash } from "node:crypto"
import type { ManagedEnvironmentOperationSummary, ManagedEnvironmentSummary } from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import type { requestManagedEnvironmentReimageRequest } from "./ipc-requests.js"
import { managedEnvironmentJson } from "./managed-environment-cloud-http.js"

type Request = Parameters<typeof requestManagedEnvironmentReimageRequest>[0]
type Admission = {environment: ManagedEnvironmentSummary & {runtimeGeneration: number}; operation: ManagedEnvironmentOperationSummary}
type Binding = Pick<ManagedEnvironmentOperationSummary, "operationId" | "idempotencyKey" | "requestDigest" | "desiredRevision">

/** MP-08 / MP-11: Cloud owns STOP reservation and convergence. Retain its
 * exact operation across polls and refresh retries before destructive reimage. */
export async function prepareManagedEnvironmentReimageStop(profile: CloudControlProfile, request: Request): Promise<void> {
  const deadline = Date.now()+25_000
  const signal = AbortSignal.timeout(25_000)
  const {environmentId, ...body} = request
  let binding: Binding | undefined
  for (;;) {
    const admission = await managedEnvironmentJson<Admission>(profile, `/managed-environments/${encodeURIComponent(environmentId)}/reimage/stop`, body, signal)
    const result = validateManagedReimageStop(admission, profile, request, binding)
    if (result.done) return
    binding = result.binding
    if (Date.now()+1_500 >= deadline) throw new Error("Managed reimage STOP is still settling; rerun confirm to resume the same operation")
    await new Promise(resolve => setTimeout(resolve, 1_500))
  }
}

export function validateManagedReimageStop({environment, operation}: Admission, profile: CloudControlProfile, request: Request, binding?: Binding): {done: boolean; binding?: Binding} {
  if (environment.environmentId !== request.environmentId || environment.accountId !== profile.accountId || operation.environmentId !== request.environmentId
    || operation.requestedByUserId !== profile.userId || !operation.operationId?.trim() || !operation.idempotencyKey?.trim() || !/^sha256:[a-f0-9]{64}$/.test(operation.requestDigest)) throw new Error("Cloud returned an unrelated reimage STOP admission")
  if (operation.kind === "reimage") {
    if ((binding && binding.requestDigest !== operation.requestDigest) || operation.idempotencyKey !== request.idempotencyKey || environment.runtimeGeneration !== request.expectedGeneration+1) throw new Error("Cloud returned an unrelated reimage replay admission")
    return {done: true}
  }
  if (operation.kind !== "stop" || operation.idempotencyKey !== `reimage-stop:${createHash("sha256").update(request.idempotencyKey).digest("hex")}`
    || environment.runtimeGeneration !== request.expectedGeneration || environment.desiredState !== "stopped" || environment.desiredRevision !== operation.desiredRevision) throw new Error("Cloud returned a stale or unrelated reimage STOP operation")
  const received = {operationId: operation.operationId, idempotencyKey: operation.idempotencyKey, requestDigest: operation.requestDigest, desiredRevision: operation.desiredRevision}
  if (binding && Object.keys(binding).some(key => binding[key as keyof Binding] !== received[key as keyof Binding])) throw new Error("Cloud changed the admitted reimage STOP operation")
  if (operation.status === "failed") throw new Error("Managed reimage STOP failed; inspect the retained lifecycle operation")
  if (operation.status !== "succeeded") return {done: false, binding: received}
  if (!operation.completedAt?.trim() || environment.observedState !== "stopped" || environment.observedRevision !== environment.desiredRevision) throw new Error("Cloud returned an unconverged completed reimage STOP")
  return {done: true, binding: received}
}
