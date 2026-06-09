import { readFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import type { WorkflowPublicationConfig } from "./publication-types.js"

export interface PublicationCloudProfile {
  readonly apiUrl: string
  readonly accountId: string
  readonly cloudSessionToken?: string
}

export interface RegisterCloudPublicationBackendInput {
  readonly deploymentId: string
  readonly publication: WorkflowPublicationConfig
  readonly localUrl?: string
  readonly status?: "ready" | "unavailable" | "failed"
  readonly lastError?: string | null
  readonly profile?: PublicationCloudProfile | null
  readonly fetch?: typeof fetch
  readonly now?: () => number
}

export interface PublicationDeploymentLogEntry {
  readonly level?: "debug" | "info" | "warn" | "error" | string
  readonly message: string
  readonly metadata?: unknown
  readonly occurredAt?: Date | string
}

export interface AppendCloudPublicationDeploymentLogsInput {
  readonly deploymentId?: string
  readonly entries: readonly PublicationDeploymentLogEntry[]
  readonly profile?: PublicationCloudProfile | null
  readonly runnerKey?: string | null
  readonly fetch?: typeof fetch
}

export async function registerCloudPublicationDeploymentBackend(
  input: RegisterCloudPublicationBackendInput,
): Promise<boolean> {
  const profile = input.profile === undefined ? await loadCloudPublicationProfile() : input.profile
  if (!profile) return false
  const fetchImpl = input.fetch ?? fetch
  const status = input.status ?? "ready"
  if (status === "ready" && !input.localUrl) {
    throw new Error("Cloud publication backend registration requires localUrl when status is ready")
  }
  const response = await fetchImpl(
    `${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(input.deploymentId)}/local-backend`,
    {
      method: "POST",
      headers: {
        accept: "application/json",
        "content-type": "application/json",
        ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
      },
      body: JSON.stringify({
        accountId: profile.accountId,
        status,
        runtimeSessionId: input.publication.session_id,
        ...(input.lastError ? { lastError: input.lastError } : {}),
        ...(status === "ready" ? {
          backendTarget: {
            kind: "local_runtime",
            url: input.localUrl,
            updated_at_ms: input.now?.() ?? Date.now(),
          },
        } : {}),
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication backend registration failed with HTTP ${response.status}: ${await response.text()}`)
  }
  return true
}

export async function appendCloudPublicationDeploymentLogs(
  input: AppendCloudPublicationDeploymentLogsInput,
): Promise<boolean> {
  const deploymentId = input.deploymentId?.trim() || process.env.ARROBA_PUBLICATION_CLOUD_DEPLOYMENT_ID?.trim()
  if (!deploymentId || input.entries.length === 0) return false
  const runnerKey = input.runnerKey ?? process.env.ARROBA_PUBLICATION_CLOUD_RUNNER_KEY?.trim() ?? null
  const fetchImpl = input.fetch ?? fetch
  if (runnerKey) {
    const apiUrl = input.profile?.apiUrl || process.env.ARROBA_PUBLICATION_CLOUD_API_URL?.trim()
    if (!apiUrl) return false
    const response = await fetchImpl(
      `${normalizeApiUrl(apiUrl)}/runner/publication-deployments/${encodeURIComponent(deploymentId)}/logs`,
      {
        method: "POST",
        headers: {
          accept: "application/json",
          "content-type": "application/json",
        },
        body: JSON.stringify({
          runnerKey,
          entries: input.entries,
        }),
      },
    )
    if (!response.ok) {
      throw new Error(`Cloud publication deployment log append failed with HTTP ${response.status}: ${await response.text()}`)
    }
    return true
  }
  const profile = input.profile === undefined ? await loadCloudPublicationProfile() : input.profile
  if (!profile) return false
  const response = await fetchImpl(
    `${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(deploymentId)}/logs`,
    {
      method: "POST",
      headers: {
        accept: "application/json",
        "content-type": "application/json",
        ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
      },
      body: JSON.stringify({
        accountId: profile.accountId,
        entries: input.entries,
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication deployment log append failed with HTTP ${response.status}: ${await response.text()}`)
  }
  return true
}

async function loadCloudPublicationProfile(): Promise<PublicationCloudProfile | null> {
  const envProfile = loadCloudPublicationProfileFromEnv()
  if (envProfile) return envProfile
  const preferences = JSON.parse(await readFile(preferencesPath(), "utf8").catch(() => "{}")) as {
    relay?: {
      cloud?: {
        apiUrl?: string
        accountId?: string
        cloudSessionToken?: string
      } | null
    }
  }
  const cloud = preferences.relay?.cloud
  if (!cloud?.apiUrl || !cloud.accountId) return null
  return {
    apiUrl: cloud.apiUrl,
    accountId: cloud.accountId,
    ...(cloud.cloudSessionToken ? { cloudSessionToken: cloud.cloudSessionToken } : {}),
  }
}

function loadCloudPublicationProfileFromEnv(): PublicationCloudProfile | null {
  const apiUrl = process.env.ARROBA_PUBLICATION_CLOUD_API_URL?.trim()
  const accountId = process.env.ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID?.trim()
  const cloudSessionToken = process.env.ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN?.trim()
  if (!apiUrl || !accountId) return null
  return {
    apiUrl,
    accountId,
    ...(cloudSessionToken ? { cloudSessionToken } : {}),
  }
}

function preferencesPath(): string {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) return path.join(xdg, "arroba", "config.json")
  return path.join(os.homedir(), ".arroba", "config.json")
}

function normalizeApiUrl(apiUrl: string): string {
  return apiUrl.trim().replace(/\/+$/, "")
}
