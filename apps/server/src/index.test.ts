import assert from "node:assert/strict"
import { createHmac } from "node:crypto"
import test from "node:test"

import {
  buildServer,
  loadPublicationConfigFromKernel,
  publicationConfigFromKernelRecord,
  type WorkflowPublicationConfig,
} from "./index.js"

const baseConfig: WorkflowPublicationConfig = {
  publication_id: "pub-test",
  session_id: "session-1",
  workflow_ref: "workflow-1",
  endpoint_ref: "endpoint-1",
  route: "/*",
  auth: { mode: "anonymous" },
  parser: { kind: "json" },
  mode: "sync",
}

test("GET /health returns an ok status payload", async () => {
  const { app } = buildServer(baseConfig, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), { status: "ok" })
  } finally {
    await app.close()
  }
})

test("gateway maps kernel-owned publication records to runtime config", async () => {
  const config = publicationConfigFromKernelRecord({
    id: "pub-1",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "public_qa",
    enabled: true,
    route: "/qa",
    methods: ["POST", "PUT"],
    auth: { mode: "bearer", token_env: "TOKEN_ENV" },
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<task>.+)$" },
    input_schema: { type: "object", required: ["task"] },
    mode: "async",
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
  }, "ws://kernel")

  assert.deepEqual(config, {
    publication_id: "pub-1",
    session_id: "session-1",
    workflow_ref: "workflow-1",
    endpoint_ref: "endpoint-1",
    kernel_endpoint: "ws://kernel",
    route: "/qa",
    methods: ["POST"],
    auth: { mode: "bearer", token_env: "TOKEN_ENV" },
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<task>.+)$" },
    input_schema: { type: "object", required: ["task"] },
    mode: "async",
  })
})

test("gateway can load publication config from kernel lookup", async () => {
  const requests: Record<string, unknown>[] = []
  const config = await loadPublicationConfigFromKernel("session-1", "pub-1", "ws://kernel", {
    send: async (request) => {
      requests.push(request)
      return {
        WorkflowPublication: {
          publication: {
            id: "pub-1",
            session_id: "session-1",
            workflow_id: "workflow-1",
            endpoint_id: "endpoint-1",
            enabled: true,
            route: "/qa",
            methods: ["GET"],
            auth: { mode: "anonymous" },
            parser: { kind: "json" },
            mode: "sync",
            created_by_user_id: "local",
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    },
  })

  assert.deepEqual(requests, [
    { GetWorkflowPublication: { session_id: "session-1", publication_ref: "pub-1" } },
  ])
  assert.equal(config.publication_id, "pub-1")
  assert.equal(config.workflow_ref, "workflow-1")
  assert.equal(config.endpoint_ref, "endpoint-1")
  assert.deepEqual(config.methods, ["GET"])
})

test("gateway authenticates, parses JSON, and forwards transport-shaped workflow output", async () => {
  process.env.GATEWAY_TEST_TOKEN = "secret"
  let seenInput: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    auth: { mode: "bearer", token_env: "GATEWAY_TEST_TOKEN" },
    input_schema: {
      type: "object",
      required: ["name"],
      properties: { name: { type: "string" } },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      return {
        accepted: true,
        workflow_run: {
          id: "run-1",
          status: "Completed",
          final_output: {
            message: JSON.stringify({
              kind: "http_response",
              status: 201,
              headers: { "content-type": "text/plain" },
              body: `hello ${(invocation.input as { name: string }).name}`,
            }),
          },
        },
      }
    },
  })

  try {
    const rejected = await app.inject({
      method: "POST",
      url: "/anything",
      payload: { name: "miguel" },
    })
    assert.equal(rejected.statusCode, 401)

    const accepted = await app.inject({
      method: "POST",
      url: "/anything",
      headers: { authorization: "Bearer secret" },
      payload: { name: "miguel" },
    })
    assert.equal(accepted.statusCode, 201)
    assert.equal(accepted.headers["content-type"], "text/plain")
    assert.equal(accepted.body, "hello miguel")
    assert.deepEqual(seenInput, { name: "miguel" })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_TOKEN
  }
})

test("gateway supports regex and path-template parsers", async () => {
  const regexInputs: unknown[] = []
  const regexServer = buildServer({
    ...baseConfig,
    parser: {
      kind: "regex",
      source: "path",
      pattern: "^/page/(?<source_path>[^/]+)/(?<instruction>.+)$",
    },
  }, {
    invokeWorkflow: async (invocation) => {
      regexInputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const response = await regexServer.app.inject({ method: "GET", url: "/page/about/make-it-green" })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(regexInputs[0], { source_path: "about", instruction: "make-it-green" })
  } finally {
    await regexServer.app.close()
  }

  const templateInputs: unknown[] = []
  const templateServer = buildServer({
    ...baseConfig,
    parser: { kind: "path_template", template: "/store/:list" },
  }, {
    invokeWorkflow: async (invocation) => {
      templateInputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-2", status: "Running" } }
    },
  })

  try {
    const response = await templateServer.app.inject({ method: "GET", url: "/store/apples%20milk" })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(templateInputs[0], { list: "apples milk" })
  } finally {
    await templateServer.app.close()
  }
})

test("gateway supports custom command parsers", async () => {
  const inputs: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    parser: {
      kind: "custom_command",
      command: process.execPath,
      args: [
        "-e",
        "let body=''; process.stdin.on('data', c => body += c); process.stdin.on('end', () => { const req = JSON.parse(body); process.stdout.write(JSON.stringify({ url: req.url, ok: true })); });",
      ],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      inputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const response = await app.inject({ method: "POST", url: "/custom", payload: { ignored: true } })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(inputs[0], { url: "/custom", ok: true })
  } finally {
    await app.close()
  }
})

test("arroba auth maps a verified connector identity to one Arroba principal", async () => {
  process.env.GATEWAY_TEST_SLACK_SECRET = "slack-secret"
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    auth: {
      mode: "arroba",
      connectors: [{ kind: "slack", signing_secret_env: "GATEWAY_TEST_SLACK_SECRET" }],
      external_identities: [{
        connector: "slack",
        external_id: "T123:U456",
        principal: {
          id: "user-miguel",
          type: "user",
          teams: ["team-core"],
          allowed_connectors: ["slack"],
        },
      }],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const badPayload = JSON.stringify({ team_id: "T123", user_id: "U456" })
    const rejected = await app.inject({
      method: "POST",
      url: "/slack",
      headers: slackHeaders("wrong", badPayload),
      payload: badPayload,
    })
    assert.equal(rejected.statusCode, 401)

    const payload = JSON.stringify({ team_id: "T123", user_id: "U456" })
    const accepted = await app.inject({
      method: "POST",
      url: "/slack",
      headers: slackHeaders("slack-secret", payload),
      payload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-miguel",
      teams: ["team-core"],
      display_name: undefined,
      allowed_connectors: ["slack"],
      proof: {
        auth: "connector",
        connector: "slack",
        external_id: "T123:U456",
        metadata: { team_id: "T123", user_id: "U456" },
      },
    })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_SLACK_SECRET
  }
})

function slackHeaders(secret: string, body: string) {
  const timestamp = String(Math.floor(Date.now() / 1000))
  return {
    "content-type": "application/json",
    "x-slack-request-timestamp": timestamp,
    "x-slack-signature": `v0=${createHmac("sha256", secret).update(`v0:${timestamp}:${body}`).digest("hex")}`,
  }
}

test("arroba auth rejects linked principals through disallowed connectors", async () => {
  const { app } = buildServer({
    ...baseConfig,
    auth: {
      mode: "arroba",
      connectors: [{ kind: "discord" }],
      external_identities: [{
        connector: "discord",
        external_id: "guild-1:user-1",
        principal: {
          id: "user-1",
          type: "user",
          allowed_connectors: ["slack"],
        },
      }],
    },
  }, {
    invokeWorkflow: async () => ({ accepted: true, workflow_run: { id: "run-1", status: "Running" } }),
  })

  try {
    const response = await app.inject({
      method: "POST",
      url: "/discord",
      headers: { "x-arroba-discord-identity": "guild-1:user-1" },
      payload: { content: "hello" },
    })
    assert.equal(response.statusCode, 401)
    assert.deepEqual(response.json(), { error: "principal is not allowed through discord" })
  } finally {
    await app.close()
  }
})

test("arroba auth keeps anonymous access explicit", async () => {
  const deniedServer = buildServer({
    ...baseConfig,
    auth: { mode: "arroba" },
  }, {
    invokeWorkflow: async () => ({ accepted: true, workflow_run: { id: "run-1", status: "Running" } }),
  })

  try {
    const denied = await deniedServer.app.inject({ method: "POST", url: "/public", payload: {} })
    assert.equal(denied.statusCode, 401)
  } finally {
    await deniedServer.app.close()
  }

  const allowedServer = buildServer({
    ...baseConfig,
    auth: { mode: "arroba", allow_anonymous: true },
  }, {
    invokeWorkflow: async (invocation) => {
      assert.deepEqual(invocation.caller, {
        type: "anonymous",
        principal_id: "anonymous",
        teams: [],
        display_name: undefined,
        allowed_connectors: undefined,
        proof: { auth: "anonymous", connector: "http" },
      })
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const allowed = await allowedServer.app.inject({ method: "POST", url: "/public", payload: {} })
    assert.equal(allowed.statusCode, 202)
  } finally {
    await allowedServer.app.close()
  }
})

test("paired sender auth is optional per publication and uses the pairing endpoint when enabled", async () => {
  const disabled = buildServer({
    ...baseConfig,
    auth: { mode: "arroba", allow_anonymous: true, paired_senders: { enabled: false } },
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })
  try {
    const response = await disabled.app.inject({
      method: "POST",
      url: "/.well-known/arroba/publication/pair",
      payload: { pair_code: "pair" },
    })
    assert.equal(response.statusCode, 404)
  } finally {
    await disabled.app.close()
  }

  const seenCallers: unknown[] = []
  const enabled = buildServer({
    ...baseConfig,
    auth: { mode: "arroba", paired_senders: { enabled: true } },
  }, {
    redeemPublicationPairCode: async (_publication, pairCode, displayName) => {
      assert.equal(pairCode, "pair-code")
      assert.equal(displayName, "sender one")
      return {
        sender: {
          sender_id: "sender-1",
          publication_id: "pub-test",
          display_name: "sender one",
          credential_hash: "hash",
          allowed_transports: ["http"],
          created_at_ms: 0,
        },
        credential: "sender-secret",
      }
    },
    authenticatePublicationSender: async (_publication, credential, transport) => {
      if (credential !== "sender-secret") throw new Error("bad sender")
      assert.equal(transport, "http")
      return {
        sender_id: "sender-1",
        publication_id: "pub-test",
        display_name: "sender one",
        credential_hash: "hash",
        allowed_transports: ["http"],
        created_at_ms: 0,
      }
    },
    invokeWorkflow: async (invocation) => {
      seenCallers.push(invocation.caller)
      return { accepted: true }
    },
  })
  try {
    const pair = await enabled.app.inject({
      method: "POST",
      url: "/.well-known/arroba/publication/pair",
      payload: { pair_code: "pair-code", display_name: "sender one" },
    })
    assert.equal(pair.statusCode, 200)
    assert.equal(pair.json().credential, "sender-secret")

    const rejected = await enabled.app.inject({
      method: "POST",
      url: "/anything",
      payload: {},
    })
    assert.equal(rejected.statusCode, 401)

    const accepted = await enabled.app.inject({
      method: "POST",
      url: "/anything",
      headers: { authorization: "Bearer sender-secret" },
      payload: {},
    })
    assert.equal(accepted.statusCode, 200)
    assert.deepEqual((seenCallers[0] as Record<string, unknown>).principal_id, "sender-1")
  } finally {
    await enabled.app.close()
  }
})
