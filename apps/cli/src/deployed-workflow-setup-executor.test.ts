import assert from "node:assert/strict"
import test from "node:test"

import { executeDeploymentSetup } from "./deployed-workflow-setup-executor.js"
import type { DeploymentSetup } from "./deployed-workflow-setup-api.js"
import type { RelayCloudProfile } from "./preferences.js"

test("deployment setup executor reloads a concurrently advanced checkpoint", async () => {
  const originalFetch = globalThis.fetch
  let getCount = 0
  let publishCount = 0
  globalThis.fetch = async (_input, init) => {
    if (init?.method === "POST") {
      return jsonResponse({
        error: {
          code: "deployment_setup_conflict",
          message: "Deployment setup changed; reload it before continuing.",
        },
      }, 409)
    }
    getCount += 1
    return jsonResponse({ setup: getCount === 1 ? activeSetup : completedSetup })
  }
  try {
    const outcome = await executeDeploymentSetup(profile, activeSetup.id, {
      publishSource: async () => {
        publishCount += 1
        return { publicationId: "publication-1", publicationDigest: digest }
      },
      exportPackage: unexpected,
      resolveProject: unexpected,
      verifyRelease: unexpected,
      credentialsReady: unexpected,
      bindRuntime: unexpected,
      activateHosted: unexpected,
    })
    assert.equal(outcome.kind, "completed")
    assert.equal(outcome.setup.version, 2)
    assert.equal(publishCount, 1)
    assert.equal(getCount, 2)
  } finally {
    globalThis.fetch = originalFetch
  }
})

async function unexpected(): Promise<never> {
  throw new Error("unexpected setup stage")
}

const digest = `sha256:${"a".repeat(64)}`

const activeSetup: DeploymentSetup = {
  id: "setup-conflict",
  accountId: "account-1",
  createdByUserId: "user-1",
  clientRequestId: "request-1",
  origin: "draft",
  status: "active",
  stage: "source",
  version: 1,
  sourceSessionId: "session-1",
  sourceWorkflowId: "workflow-1",
  sourceWorkflowRevision: "7",
  configuration: {
    endpointId: "endpoint-1",
    publication: { alias: "demo-r7", kind: "ingress" },
    deployment: {
      name: "Demo",
      slug: "demo",
      kind: "workflow_endpoint",
      runtimeMode: "hosted_container",
      region: "eu-central",
    },
  },
  createdAt: "2026-07-18T00:00:00.000Z",
  updatedAt: "2026-07-18T00:00:00.000Z",
  operationKeys: {
    publication: "publication-key",
    package: "package-key",
    project: "project-key",
    release: "release-key",
    credentials: "credentials-key",
    promotion: "promotion-key",
    runtime: "runtime-key",
  },
}

const completedSetup: DeploymentSetup = {
  ...activeSetup,
  status: "completed",
  stage: "complete",
  version: 2,
  sourcePublicationId: "publication-1",
  sourcePublicationDigest: digest,
  completedAt: "2026-07-18T00:00:01.000Z",
}

const profile: RelayCloudProfile = {
  apiUrl: "https://chariox-cloud-staging.osc-fr1.scalingo.io",
  email: "user@example.test",
  accountId: "account-1",
  userId: "user-1",
  accountSlug: "account",
  realmId: "realm-1",
  relayUrl: "wss://relay.example.test",
  issuerId: "issuer-1",
  cloudSessionToken: "session-token",
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  })
}
