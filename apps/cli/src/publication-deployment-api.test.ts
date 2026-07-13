import assert from "node:assert/strict"
import { mkdtemp, writeFile, rm } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"

import {
  createPublicationDeploymentFromPackage,
  readPublicationPackageMetadata,
  reuploadPublicationDeploymentPackage,
} from "./publication-deployment-api.js"
import type { RelayCloudProfile } from "./preferences.js"

test("publication deployment API reads package metadata", async () => {
  const root = await publicationPackageFixture()
  try {
    const metadata = await readPublicationPackageMetadata(root)
    assert.equal(metadata.publicationId, "pub-1")
    assert.equal(metadata.publicationAlias, "Public Demo")
    assert.equal(metadata.workflowId, "workflow-1")
    assert.equal(metadata.endpointId, "endpoint-1")
    assert.equal(metadata.hookId, "hook-1")
    assert.equal(metadata.transport, "human_http")
    assert.equal(metadata.route, "/final/*")
    assert.equal(metadata.packageUri, `file://${root}`)
    assert.deepEqual(metadata.agentApp, {
      enabled: true,
      routes: [{ path: "/add/*", prompt_source: "path_tail" }],
      replicas: { count: 2 },
    })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication deployment API creates, uploads, and starts hosted deployments", async () => {
  const root = await publicationPackageFixture()
  const previousFetch = globalThis.fetch
  const calls: Array<{ readonly url: string; readonly method: string; readonly body: unknown }> = []
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = input instanceof Request ? input.url : String(input)
    const method = init?.method ?? "GET"
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : null
    calls.push({ url, method, body })
    if (url.endsWith("/publication-deployments")) {
      return jsonResponse({ deployment: deploymentPayload({ id: "deployment-1", status: "pending" }) })
    }
    if (url.endsWith("/publication-deployments/deployment-1/package")) {
      return jsonResponse({ deployment: deploymentPayload({ id: "deployment-1", status: "package_uploaded", packageUri: body.packageUri }) })
    }
    if (url.endsWith("/publication-deployments/deployment-1/start")) {
      return jsonResponse({ jobs: [{ id: "job-1" }] }, 202)
    }
    return jsonResponse({ error: { message: `unexpected ${url}` } }, 404)
  }) as typeof fetch

  try {
    const deployment = await createPublicationDeploymentFromPackage({
      profile: profile(),
      packagePath: root,
      mode: "hosted_container",
      credentialProfile: "miguel_staging",
    })

    assert.equal(deployment.id, "deployment-1")
    assert.deepEqual(calls.map((call) => [call.method, new URL(call.url).pathname]), [
      ["POST", "/publication-deployments"],
      ["POST", "/publication-deployments/deployment-1/package"],
      ["POST", "/publication-deployments/deployment-1/start"],
    ])
    assert.equal("createdByUserId" in (calls[0]?.body as Record<string, unknown>), false)
    assert.equal((calls[0]?.body as Record<string, unknown>).credentialProfile, "miguel_staging")
    assert.equal((calls[0]?.body as Record<string, unknown>).route, "/final/*")
    assert.deepEqual((calls[0]?.body as Record<string, unknown>).agentApp, {
      enabled: true,
      routes: [{ path: "/add/*", prompt_source: "path_tail" }],
      replicas: { count: 2 },
    })
    assert.equal((calls[1]?.body as Record<string, unknown>).packageUri, `file://${root}`)
    assert.match(String((calls[1]?.body as Record<string, unknown>).packageDigest), /^sha256:/)
  } finally {
    globalThis.fetch = previousFetch
    await rm(root, { recursive: true, force: true })
  }
})

test("publication deployment API reuploads package archives", async () => {
  const root = await publicationPackageFixture()
  const previousFetch = globalThis.fetch
  const calls: Array<{ readonly url: string; readonly method: string; readonly body: unknown }> = []
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = input instanceof Request ? input.url : String(input)
    const method = init?.method ?? "GET"
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : null
    calls.push({ url, method, body })
    if (url.endsWith("/publication-deployments/deployment-1/package")) {
      return jsonResponse({ deployment: deploymentPayload({ id: "deployment-1", status: "package_uploaded", packageUri: body.packageUri }) })
    }
    return jsonResponse({ error: { message: `unexpected ${url}` } }, 404)
  }) as typeof fetch

  try {
    const deployment = await reuploadPublicationDeploymentPackage({
      profile: profile(),
      deploymentId: "deployment-1",
      packagePath: root,
    })

    assert.equal(deployment.id, "deployment-1")
    assert.deepEqual(calls.map((call) => [call.method, new URL(call.url).pathname]), [
      ["POST", "/publication-deployments/deployment-1/package"],
    ])
    assert.equal((calls[0]?.body as Record<string, unknown>).packageUri, `file://${root}`)
    assert.match(String((calls[0]?.body as Record<string, unknown>).packageDigest), /^sha256:/)
    assert.equal(typeof (calls[0]?.body as Record<string, unknown>).packageArchiveBase64, "string")
  } finally {
    globalThis.fetch = previousFetch
    await rm(root, { recursive: true, force: true })
  }
})

test("managed Cloud deployment rejects persistent patch packages before network access", async () => {
  const root = await publicationPackageFixture({
    enabled: true,
    persistent_patch: { enabled: true },
    routes: [{ path: "/admin/*", manipulation: { level: "persistent_patch", scope: "persistent" } }],
  })
  const previousFetch = globalThis.fetch
  let fetchCalls = 0
  globalThis.fetch = (async () => {
    fetchCalls += 1
    throw new Error("unexpected fetch")
  }) as typeof fetch

  try {
    await assert.rejects(
      createPublicationDeploymentFromPackage({
        profile: profile(),
        packagePath: root,
        mode: "hosted_container",
      }),
      /Persistent patches are not available for managed Cloud deployments/,
    )
    await assert.rejects(
      reuploadPublicationDeploymentPackage({
        profile: profile(),
        deploymentId: "deployment-1",
        packagePath: root,
      }),
      /Persistent patches are not available for managed Cloud deployments/,
    )
    assert.equal(fetchCalls, 0)
  } finally {
    globalThis.fetch = previousFetch
    await rm(root, { recursive: true, force: true })
  }
})

async function publicationPackageFixture(agentApp: unknown = {
  enabled: true,
  routes: [{ path: "/add/*", prompt_source: "path_tail" }],
  replicas: { count: 2 },
}): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-package-"))
  await writeFile(join(root, "publication.json"), JSON.stringify({
    schema_version: 1,
    package_version: 1,
    publication_id: "pub-1",
    alias: "Public Demo",
    workflow_id: "workflow-1",
    hooks: [{
      id: "hook-1",
      endpoint_id: "endpoint-1",
      transport: "human_http",
      route: "/final/*",
    }],
    agent_app: agentApp,
  }, null, 2))
  return root
}

function profile(): RelayCloudProfile {
  return {
    apiUrl: "https://cloud.example.test",
    email: "user@example.test",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "account",
    realmId: "realm-1",
    relayUrl: "wss://relay.example.test",
    issuerId: "issuer-1",
    cloudSessionToken: "session-token",
  }
}

function deploymentPayload(overrides: Partial<Record<string, unknown>> = {}): Record<string, unknown> {
  return {
    id: "deployment-1",
    mode: "hosted_container",
    slug: "pub-1",
    publicBaseUrl: "https://publication.example.test/pub-1/",
    status: "ready",
    publicationId: "pub-1",
    transport: "human_http",
    ...overrides,
  }
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
