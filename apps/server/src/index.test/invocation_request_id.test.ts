import { createHmac } from "node:crypto"

import {
  assert,
  baseConfig,
  buildServer,
  test,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"
import { configurePublicationCallerClaimsRuntimeForTests } from "../publication-caller-claims.js"
import type { NormalizedInvocation } from "../publication-types.js"

const CALLER_CLAIMS_SECRET = "caller-claims-runtime-secret-0123456789"
const CALLER_CLAIMS_NOW_SECONDS = Math.floor(Date.parse("2026-07-15T12:00:00.000Z") / 1_000)

const humanHttpConfig: WorkflowPublicationConfig = {
  ...baseConfig,
  transport: "human_http",
  route: "/",
  methods: ["GET", "POST"],
  parser: { kind: "query_params" },
  mode: "async",
}

function configureRuntime() {
  configurePublicationCallerClaimsRuntimeForTests({
    deploymentId: "deployment-1",
    environmentId: "environment-1",
    secret: CALLER_CLAIMS_SECRET,
    now: () => new Date(CALLER_CLAIMS_NOW_SECONDS * 1_000),
  })
}

function signedCallerClaimsHeaders(options: {
  readonly invocationId: string
  readonly nonce: string
  readonly subject?: string
  readonly roles?: readonly string[]
}): Record<string, string> {
  const encodedHeader = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const encodedPayload = Buffer.from(JSON.stringify({
    iss: "chariox-cloud",
    aud: "deployment-1",
    sub: options.subject ?? "user:user-1",
    org: "account-1",
    roles: options.roles ?? ["public"],
    deployment_id: "deployment-1",
    environment_id: "environment-1",
    invocation_id: options.invocationId,
    nonce: options.nonce,
    iat: CALLER_CLAIMS_NOW_SECONDS,
    exp: CALLER_CLAIMS_NOW_SECONDS + 60,
  })).toString("base64url")
  const unsigned = `${encodedHeader}.${encodedPayload}`
  const token = `${unsigned}.${createHmac("sha256", CALLER_CLAIMS_SECRET).update(unsigned).digest("base64url")}`
  return {
    "x-chariox-caller-claims": token,
    "x-chariox-invocation-id": options.invocationId,
  }
}

function captureServer() {
  const invocations: NormalizedInvocation[] = []
  const { app } = buildServer(humanHttpConfig, {
    invokeWorkflow: async (invocation) => {
      invocations.push(invocation)
      return {
        accepted: true,
        workflow_run: { id: `run-${invocations.length}`, status: "Completed", final_output: { message: "ok" } },
      }
    },
  })
  return { app, invocations }
}

test("regular human HTTP reuses the verified caller invocation id as the request id", async () => {
  configureRuntime()
  const { app, invocations } = captureServer()
  try {
    const response = await app.inject({
      method: "GET",
      url: "/?prompt=review",
      headers: {
        accept: "text/html",
        ...signedCallerClaimsHeaders({ invocationId: "invocation-http-auth", nonce: "nonce-http-auth" }),
      },
    })
    assert.equal(response.statusCode, 200)
    assert.equal(invocations.length, 1)
    assert.equal(invocations[0]?.request_id, "invocation-http-auth")
    assert.equal((invocations[0]?.caller as { type?: string })?.type, "authenticated")
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})

test("regular human HTTP mints a req_ id for anonymous callers", async () => {
  configurePublicationCallerClaimsRuntimeForTests(null)
  const { app, invocations } = captureServer()
  try {
    const response = await app.inject({ method: "GET", url: "/?prompt=review", headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 200)
    assert.equal(invocations.length, 1)
    assert.match(invocations[0]?.request_id ?? "", /^req_/)
    assert.equal((invocations[0]?.caller as { type?: string })?.type, "anonymous")
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})

test("regular human HTTP rejects an invocation id that does not match the signed claim", async () => {
  configureRuntime()
  const { app, invocations } = captureServer()
  try {
    const response = await app.inject({
      method: "GET",
      url: "/?prompt=review",
      headers: {
        accept: "text/html",
        ...signedCallerClaimsHeaders({ invocationId: "invocation-signed", nonce: "nonce-mismatch" }),
        "x-chariox-invocation-id": "invocation-attacker",
      },
    })
    assert.equal(response.statusCode, 401)
    assert.equal(invocations.length, 0)
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})

test("fixed form POST reuses the verified caller invocation id as the request id", async () => {
  configureRuntime()
  const { app, invocations } = captureServer()
  try {
    const response = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/publication/human-http/invoke",
      headers: {
        "content-type": "application/json",
        ...signedCallerClaimsHeaders({ invocationId: "invocation-form-auth", nonce: "nonce-form-auth" }),
      },
      payload: { prompt: "ship it", artifacts: [] },
    })
    assert.equal(response.statusCode, 200)
    assert.equal(invocations.length, 1)
    assert.equal(invocations[0]?.request_id, "invocation-form-auth")
    assert.equal((invocations[0]?.caller as { type?: string })?.type, "authenticated")
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})

test("fixed form POST mints a req_ id for anonymous callers", async () => {
  configurePublicationCallerClaimsRuntimeForTests(null)
  const { app, invocations } = captureServer()
  try {
    const response = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/publication/human-http/invoke",
      headers: { "content-type": "application/json" },
      payload: { prompt: "ship it", artifacts: [] },
    })
    assert.equal(response.statusCode, 200)
    assert.equal(invocations.length, 1)
    assert.match(invocations[0]?.request_id ?? "", /^req_/)
    assert.equal((invocations[0]?.caller as { type?: string })?.type, "anonymous")
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})
