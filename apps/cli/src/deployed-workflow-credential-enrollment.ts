import type {
  DeploymentCredentialEnrollmentArmedResponse,
} from "@arroba/kernel-client"
import {
  parseAbsoluteInstantMsOrNull,
} from "@arroba/kernel-client/time"

import { armDeploymentCredentialEnrollmentRequest } from "./ipc-requests.js"
import type {
  DeploymentCredentialCallbackChannelResult,
  DeploymentCredentialEnrollmentSummary,
  DeploymentCredentialProfileSummary,
} from "./deployed-workflow-types.js"

export const CLAUDE_CREDENTIAL_ENROLLMENT_PROTOCOL_VERSION = 241

export interface ClaudeCredentialEnrollmentBinding {
  readonly accountId: string
  readonly enrollmentId: string
  readonly profileId: string
  readonly targetVersion: number
  readonly enrollmentExpiresAt: string
  readonly realmId: string
  readonly kernelTarget: string
  readonly sessionId: string
  readonly attachmentId: string
  readonly agentId: string
}

export interface ClaudeCredentialEnrollmentArmDeps {
  readonly sendKernelRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  readonly armCloudCallbackChannel: (
    binding: Omit<ClaudeCredentialEnrollmentBinding, "attachmentId" | "enrollmentExpiresAt">,
  ) => Promise<DeploymentCredentialCallbackChannelResult>
  readonly now?: () => number
  readonly cloudRetryDelayMs?: number
}

export function isClaudeCredentialProfile(
  profile: Pick<DeploymentCredentialProfileSummary, "kind" | "provider">,
): boolean {
  if (profile.kind !== "provider") return false
  const provider = profile.provider?.trim().toLowerCase()
  return provider === "claude" || provider === "claude-p" || provider === "claude-headless"
}

export function requiresClaudeCredentialCallbackChannel(
  profile: Pick<DeploymentCredentialProfileSummary, "kind" | "provider">,
  enrollment: DeploymentCredentialEnrollmentSummary | null | undefined,
): enrollment is DeploymentCredentialEnrollmentSummary {
  return isClaudeCredentialProfile(profile)
    && enrollment?.mode === "provider_native"
    && (enrollment.status === "pending" || enrollment.status === "claimed")
}

export async function armClaudeCredentialEnrollment(
  deps: ClaudeCredentialEnrollmentArmDeps,
  binding: ClaudeCredentialEnrollmentBinding,
): Promise<void> {
  const now = deps.now?.() ?? Date.now()
  validateBinding(binding, now)
  const kernelRequest = armDeploymentCredentialEnrollmentRequest(
    binding.sessionId,
    binding.attachmentId,
    binding.agentId,
    binding.enrollmentId,
    binding.profileId,
    binding.targetVersion,
  )
  let kernelResponse: Record<string, unknown>
  try {
    kernelResponse = await deps.sendKernelRequest(kernelRequest)
  } catch (error) {
    throw new Error(
      `kernel protocol ${CLAUDE_CREDENTIAL_ENROLLMENT_PROTOCOL_VERSION} credential enrollment arm failed: ${errorMessage(error)}`,
    )
  }
  requireExactKernelArm(kernelResponse, binding, now)

  const cloudBinding = {
    accountId: binding.accountId,
    enrollmentId: binding.enrollmentId,
    profileId: binding.profileId,
    targetVersion: binding.targetVersion,
    realmId: binding.realmId,
    kernelTarget: binding.kernelTarget,
    sessionId: binding.sessionId,
    agentId: binding.agentId,
  }
  const cloudResponse = await armCloudWithRetry(deps, cloudBinding)
  requireExactCloudArm(cloudResponse, binding, now)
}

async function armCloudWithRetry(
  deps: ClaudeCredentialEnrollmentArmDeps,
  binding: Omit<ClaudeCredentialEnrollmentBinding, "attachmentId" | "enrollmentExpiresAt">,
): Promise<DeploymentCredentialCallbackChannelResult> {
  try {
    return await deps.armCloudCallbackChannel(binding)
  } catch {}
  const delayMs = Math.max(0, deps.cloudRetryDelayMs ?? 150)
  if (delayMs > 0) await delay(delayMs)
  try {
    return await deps.armCloudCallbackChannel(binding)
  } catch (error) {
    throw new Error(
      `Cloud credential callback channel arm failed after an exact-binding retry: ${errorMessage(error)}`,
    )
  }
}

function requireExactKernelArm(
  response: Record<string, unknown>,
  binding: ClaudeCredentialEnrollmentBinding,
  now: number,
): void {
  const armed = objectRecord(response.DeploymentCredentialEnrollmentArmed)
  if (!armed) {
    throw new Error(
      `kernel protocol ${CLAUDE_CREDENTIAL_ENROLLMENT_PROTOCOL_VERSION} did not return DeploymentCredentialEnrollmentArmed`,
    )
  }
  const expected: DeploymentCredentialEnrollmentArmedResponse["DeploymentCredentialEnrollmentArmed"] = {
    enrollment_id: binding.enrollmentId,
    profile_id: binding.profileId,
    target_version: binding.targetVersion,
    session_id: binding.sessionId,
    agent_id: binding.agentId,
    expires_at_ms: Number(armed.expires_at_ms),
  }
  for (const [field, value] of Object.entries(expected)) {
    if (field === "expires_at_ms") continue
    if (armed[field] !== value) {
      throw new Error(`kernel returned a mismatched credential enrollment arm (${field})`)
    }
  }
  if (!Number.isSafeInteger(armed.expires_at_ms) || Number(armed.expires_at_ms) <= now) {
    throw new Error("kernel returned a stale credential enrollment arm")
  }
}

function requireExactCloudArm(
  response: DeploymentCredentialCallbackChannelResult,
  binding: ClaudeCredentialEnrollmentBinding,
  now: number,
): void {
  const channel = objectRecord(response?.channel)
  if (!channel || channel.status !== "armed") {
    throw new Error("Cloud did not return an armed credential callback channel")
  }
  const expected: Readonly<Record<string, string | number>> = {
    accountId: binding.accountId,
    enrollmentId: binding.enrollmentId,
    profileId: binding.profileId,
    targetVersion: binding.targetVersion,
    realmId: binding.realmId,
    kernelTarget: binding.kernelTarget,
    sessionId: binding.sessionId,
    agentId: binding.agentId,
  }
  for (const [field, value] of Object.entries(expected)) {
    if (channel[field] !== value) {
      throw new Error(`Cloud returned a mismatched credential callback channel (${field})`)
    }
  }
  const armedAt = parseAbsoluteInstantMsOrNull(String(channel.armedAt ?? ""))
  const expiresAt = parseAbsoluteInstantMsOrNull(String(channel.expiresAt ?? ""))
  const enrollmentExpiresAt = parseAbsoluteInstantMsOrNull(binding.enrollmentExpiresAt)
  if (
    armedAt === null
    || expiresAt === null
    || enrollmentExpiresAt === null
    || expiresAt !== enrollmentExpiresAt
    || armedAt >= expiresAt
    || expiresAt <= now
  ) {
    throw new Error("Cloud returned a stale credential callback channel")
  }
}

function validateBinding(binding: ClaudeCredentialEnrollmentBinding, now: number): void {
  for (const [field, value] of Object.entries(binding)) {
    if (field === "targetVersion") continue
    if (typeof value !== "string" || !value.trim()) {
      throw new Error(`credential enrollment binding ${field} is missing`)
    }
  }
  if (!Number.isSafeInteger(binding.targetVersion) || binding.targetVersion < 1) {
    throw new Error("credential enrollment target version is invalid")
  }
  if (
    (parseAbsoluteInstantMsOrNull(binding.enrollmentExpiresAt) ?? 0) <= now
  ) {
    throw new Error("credential enrollment expiry is invalid")
  }
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds))
}
