import type { CloudClientCredential } from "./cloud-client-credential-store.js"

// MP-08 / MP-11: private routing hints, never membership or account authority.
export type CloudClientSessionScope = {caller: string; sessionId: string; accountId: string}

export function cloudClientCallerScope(credential: CloudClientCredential): string {
  const {apiUrl, accountId, userId} = credential.profile
  return JSON.stringify([new URL(apiUrl).origin, accountId, userId, credential.clientId, credential.publicKeyThumbprint])
}

export function cloudClientSessionScopes(credential: CloudClientCredential): CloudClientSessionScope[] {
  const caller = cloudClientCallerScope(credential)
  return (credential.collaborationScopes ?? []).filter(scope => scope.caller === caller)
}

export function validCloudClientSessionScopes(value: unknown): boolean {
  return value === undefined || (Array.isArray(value) && value.every(scope => scope
    && [scope.caller, scope.sessionId, scope.accountId].every(field => typeof field === "string" && field.length > 0)))
}
