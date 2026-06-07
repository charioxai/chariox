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
  readonly localUrl: string
  readonly profile?: PublicationCloudProfile | null
  readonly fetch?: typeof fetch
  readonly now?: () => number
}

export async function registerCloudPublicationDeploymentBackend(
  input: RegisterCloudPublicationBackendInput,
): Promise<boolean> {
  const profile = input.profile === undefined ? await loadCloudPublicationProfile() : input.profile
  if (!profile) return false
  const fetchImpl = input.fetch ?? fetch
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
        status: "ready",
        runtimeSessionId: input.publication.session_id,
        backendTarget: {
          kind: "local_runtime",
          url: input.localUrl,
          updated_at_ms: input.now?.() ?? Date.now(),
        },
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication backend registration failed with HTTP ${response.status}: ${await response.text()}`)
  }
  return true
}

async function loadCloudPublicationProfile(): Promise<PublicationCloudProfile | null> {
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

function preferencesPath(): string {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) return path.join(xdg, "arroba", "config.json")
  return path.join(os.homedir(), ".arroba", "config.json")
}

function normalizeApiUrl(apiUrl: string): string {
  return apiUrl.trim().replace(/\/+$/, "")
}
