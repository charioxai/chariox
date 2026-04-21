import assert from "node:assert/strict"
import { createHmac, generateKeyPairSync, sign, type KeyObject } from "node:crypto"
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

test("gateway returns HTTP 400 for parser and schema failures", async () => {
  let invoked = false
  const parserServer = buildServer({
    ...baseConfig,
    parser: { kind: "regex", source: "path", pattern: "^/ok/(?<value>.+)$" },
  }, {
    invokeWorkflow: async () => {
      invoked = true
      return { accepted: true }
    },
  })
  try {
    const response = await parserServer.app.inject({ method: "GET", url: "/bad/value" })
    assert.equal(response.statusCode, 400)
    assert.match(response.json().error, /did not match/)
    assert.equal(invoked, false)
  } finally {
    await parserServer.app.close()
  }

  const schemaServer = buildServer({
    ...baseConfig,
    input_schema: { type: "object", required: ["name"], properties: { name: { type: "string" } } },
  }, {
    invokeWorkflow: async () => {
      invoked = true
      return { accepted: true }
    },
  })
  try {
    const response = await schemaServer.app.inject({ method: "POST", url: "/schema", payload: { name: 42 } })
    assert.equal(response.statusCode, 400)
    assert.match(response.json().error, /field name expected string/)
    assert.equal(invoked, false)
  } finally {
    await schemaServer.app.close()
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

test("slack connector handles signed URL verification without invoking workflow", async () => {
  process.env.GATEWAY_TEST_SLACK_SECRET = "slack-secret"
  let invoked = false
  const { app } = buildServer({
    ...baseConfig,
    methods: ["POST"],
    auth: {
      mode: "arroba",
      connectors: [{ kind: "slack", signing_secret_env: "GATEWAY_TEST_SLACK_SECRET" }],
    },
  }, {
    invokeWorkflow: async () => {
      invoked = true
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const payload = JSON.stringify({ type: "url_verification", challenge: "challenge-token" })
    const accepted = await app.inject({
      method: "POST",
      url: "/slack/events",
      headers: slackHeaders("slack-secret", payload),
      payload,
    })
    assert.equal(accepted.statusCode, 200)
    assert.equal(accepted.body, "challenge-token")
    assert.equal(invoked, false)

    const rejected = await app.inject({
      method: "POST",
      url: "/slack/events",
      headers: slackHeaders("wrong-secret", payload),
      payload,
    })
    assert.equal(rejected.statusCode, 401)
    assert.equal(invoked, false)
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_SLACK_SECRET
  }
})

test("slack connector accepts signed slash-command form payloads", async () => {
  process.env.GATEWAY_TEST_SLACK_SECRET = "slack-secret"
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    methods: ["POST"],
    auth: {
      mode: "arroba",
      connectors: [{ kind: "slack", signing_secret_env: "GATEWAY_TEST_SLACK_SECRET" }],
      external_identities: [{
        connector: "slack",
        external_id: "T123:U456",
        principal: { id: "user-miguel", type: "user", allowed_connectors: ["slack"] },
      }],
    },
    parser: { kind: "webhook" },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const payload = new URLSearchParams({
      team_id: "T123",
      user_id: "U456",
      command: "/arroba",
      text: "ship it",
    }).toString()
    const accepted = await app.inject({
      method: "POST",
      url: "/slack/commands",
      headers: slackHeaders("slack-secret", payload, "application/x-www-form-urlencoded"),
      payload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-miguel",
      teams: [],
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

test("telegram connector verifies webhook secret and maps sender identity", async () => {
  process.env.GATEWAY_TEST_TELEGRAM_SECRET = "telegram-secret"
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    methods: ["POST"],
    auth: {
      mode: "arroba",
      connectors: [{ kind: "telegram", webhook_secret_env: "GATEWAY_TEST_TELEGRAM_SECRET" }],
      external_identities: [{
        connector: "telegram",
        external_id: "456",
        principal: { id: "user-telegram", type: "user", allowed_connectors: ["telegram"] },
      }],
    },
    parser: { kind: "webhook" },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const payload = {
      update_id: 123,
      message: {
        message_id: 1,
        chat: { id: 789, type: "private" },
        from: { id: 456, username: "miguel" },
        text: "ship it",
      },
    }
    const rejected = await app.inject({
      method: "POST",
      url: "/telegram/webhook",
      headers: { "x-telegram-bot-api-secret-token": "wrong-secret" },
      payload,
    })
    assert.equal(rejected.statusCode, 401)
    assert.equal(callers.length, 0)

    const accepted = await app.inject({
      method: "POST",
      url: "/telegram/webhook",
      headers: { "x-telegram-bot-api-secret-token": "telegram-secret" },
      payload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-telegram",
      teams: [],
      display_name: undefined,
      allowed_connectors: ["telegram"],
      proof: {
        auth: "connector",
        connector: "telegram",
        external_id: "456",
        metadata: { username: "miguel", chat_id: "789" },
      },
    })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_TELEGRAM_SECRET
  }
})

test("discord connector verifies Ed25519 signatures, handles ping, and maps interaction identity", async () => {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519")
  process.env.GATEWAY_TEST_DISCORD_PUBLIC_KEY = discordPublicKeyHex(publicKey)
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    methods: ["POST"],
    auth: {
      mode: "arroba",
      connectors: [{ kind: "discord", public_key_env: "GATEWAY_TEST_DISCORD_PUBLIC_KEY" }],
      external_identities: [{
        connector: "discord",
        external_id: "guild-1:user-1",
        principal: { id: "user-discord", type: "user", allowed_connectors: ["discord"] },
      }],
    },
    parser: { kind: "webhook" },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const pingPayload = JSON.stringify({ type: 1 })
    const ping = await app.inject({
      method: "POST",
      url: "/discord/interactions",
      headers: discordHeaders(privateKey, pingPayload),
      payload: pingPayload,
    })
    assert.equal(ping.statusCode, 200)
    assert.deepEqual(ping.json(), { type: 1 })
    assert.equal(callers.length, 0)

    const interactionPayload = JSON.stringify({
      type: 2,
      guild_id: "guild-1",
      member: { user: { id: "user-1", username: "miguel" } },
      data: { name: "arroba", options: [{ name: "prompt", value: "ship it" }] },
    })
    const rejected = await app.inject({
      method: "POST",
      url: "/discord/interactions",
      headers: discordHeaders(privateKey, interactionPayload, { tamperSignature: true }),
      payload: interactionPayload,
    })
    assert.equal(rejected.statusCode, 401)

    const accepted = await app.inject({
      method: "POST",
      url: "/discord/interactions",
      headers: discordHeaders(privateKey, interactionPayload),
      payload: interactionPayload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-discord",
      teams: [],
      display_name: undefined,
      allowed_connectors: ["discord"],
      proof: {
        auth: "connector",
        connector: "discord",
        external_id: "guild-1:user-1",
        metadata: { guild_id: "guild-1", user_id: "user-1", username: "miguel" },
      },
    })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_DISCORD_PUBLIC_KEY
  }
})

test("whatsapp connector verifies Meta webhook challenge, HMAC, and sender identity", async () => {
  process.env.GATEWAY_TEST_WHATSAPP_APP_SECRET = "whatsapp-app-secret"
  process.env.GATEWAY_TEST_WHATSAPP_VERIFY_TOKEN = "verify-token"
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    methods: ["GET", "POST"],
    auth: {
      mode: "arroba",
      connectors: [{
        kind: "whatsapp",
        app_secret_env: "GATEWAY_TEST_WHATSAPP_APP_SECRET",
        verify_token_env: "GATEWAY_TEST_WHATSAPP_VERIFY_TOKEN",
      }],
      external_identities: [{
        connector: "whatsapp",
        external_id: "15551234567",
        principal: { id: "user-whatsapp", type: "user", allowed_connectors: ["whatsapp"] },
      }],
    },
    parser: { kind: "webhook" },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const challenge = await app.inject({
      method: "GET",
      url: "/whatsapp/webhook?hub.mode=subscribe&hub.verify_token=verify-token&hub.challenge=challenge-1",
    })
    assert.equal(challenge.statusCode, 200)
    assert.equal(challenge.body, "challenge-1")
    assert.equal(callers.length, 0)

    const rejectedChallenge = await app.inject({
      method: "GET",
      url: "/whatsapp/webhook?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=challenge-1",
    })
    assert.equal(rejectedChallenge.statusCode, 401)

    const payload = JSON.stringify(whatsAppPayload())
    const rejected = await app.inject({
      method: "POST",
      url: "/whatsapp/webhook",
      headers: whatsAppHeaders("wrong-secret", payload),
      payload,
    })
    assert.equal(rejected.statusCode, 401)

    const accepted = await app.inject({
      method: "POST",
      url: "/whatsapp/webhook",
      headers: whatsAppHeaders("whatsapp-app-secret", payload),
      payload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-whatsapp",
      teams: [],
      display_name: undefined,
      allowed_connectors: ["whatsapp"],
      proof: {
        auth: "connector",
        connector: "whatsapp",
        external_id: "15551234567",
        metadata: { phone_number_id: "phone-number-id", display_phone_number: "15557654321" },
      },
    })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_WHATSAPP_APP_SECRET
    delete process.env.GATEWAY_TEST_WHATSAPP_VERIFY_TOKEN
  }
})

test("signal connector verifies bridge webhook secret and sender identity", async () => {
  process.env.GATEWAY_TEST_SIGNAL_SECRET = "signal-secret"
  const callers: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    methods: ["POST"],
    auth: {
      mode: "arroba",
      connectors: [{ kind: "signal", webhook_secret_env: "GATEWAY_TEST_SIGNAL_SECRET" }],
      external_identities: [{
        connector: "signal",
        external_id: "signal-source-uuid",
        principal: { id: "user-signal", type: "user", allowed_connectors: ["signal"] },
      }],
    },
    parser: { kind: "webhook" },
  }, {
    invokeWorkflow: async (invocation) => {
      callers.push(invocation.caller)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const payload = {
      envelope: {
        sourceUuid: "signal-source-uuid",
        sourceNumber: "+15551234567",
        dataMessage: { message: "ship it" },
      },
    }
    const rejected = await app.inject({
      method: "POST",
      url: "/signal/webhook",
      headers: { "x-signal-webhook-secret": "wrong-secret" },
      payload,
    })
    assert.equal(rejected.statusCode, 401)
    assert.equal(callers.length, 0)

    const accepted = await app.inject({
      method: "POST",
      url: "/signal/webhook",
      headers: { "x-signal-webhook-secret": "signal-secret" },
      payload,
    })
    assert.equal(accepted.statusCode, 202)
    assert.deepEqual(callers[0], {
      type: "user",
      principal_id: "user-signal",
      teams: [],
      display_name: undefined,
      allowed_connectors: ["signal"],
      proof: {
        auth: "connector",
        connector: "signal",
        external_id: "signal-source-uuid",
        metadata: { source_number: "+15551234567", source_uuid: "signal-source-uuid" },
      },
    })
  } finally {
    await app.close()
    delete process.env.GATEWAY_TEST_SIGNAL_SECRET
  }
})

function slackHeaders(secret: string, body: string, contentType = "application/json") {
  const timestamp = String(Math.floor(Date.now() / 1000))
  return {
    "content-type": contentType,
    "x-slack-request-timestamp": timestamp,
    "x-slack-signature": `v0=${createHmac("sha256", secret).update(`v0:${timestamp}:${body}`).digest("hex")}`,
  }
}

function discordHeaders(
  privateKey: KeyObject,
  body: string,
  options: { tamperSignature?: boolean } = {},
) {
  const timestamp = String(Math.floor(Date.now() / 1000))
  const signature = sign(null, Buffer.from(`${timestamp}${body}`), privateKey).toString("hex")
  return {
    "content-type": "application/json",
    "x-signature-timestamp": timestamp,
    "x-signature-ed25519": options.tamperSignature ? `00${signature.slice(2)}` : signature,
  }
}

function discordPublicKeyHex(publicKey: KeyObject) {
  const der = publicKey.export({ format: "der", type: "spki" }) as Buffer
  return der.subarray(-32).toString("hex")
}

function whatsAppHeaders(secret: string, body: string) {
  return {
    "content-type": "application/json",
    "x-hub-signature-256": `sha256=${createHmac("sha256", secret).update(body).digest("hex")}`,
  }
}

function whatsAppPayload() {
  return {
    object: "whatsapp_business_account",
    entry: [{
      id: "business-id",
      changes: [{
        field: "messages",
        value: {
          messaging_product: "whatsapp",
          metadata: {
            display_phone_number: "15557654321",
            phone_number_id: "phone-number-id",
          },
          contacts: [{ wa_id: "15551234567", profile: { name: "Miguel" } }],
          messages: [{ from: "15551234567", id: "wamid.1", text: { body: "ship it" }, type: "text" }],
        },
      }],
    }],
  }
}

test("arroba auth rejects linked principals through disallowed connectors", async () => {
  const { app } = buildServer({
    ...baseConfig,
    auth: {
      mode: "arroba",
      connectors: [{ kind: "http", principal: { id: "http-subject" } }],
      external_identities: [{
        connector: "http",
        external_id: "http-subject",
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
      url: "/http",
      payload: { content: "hello" },
    })
    assert.equal(response.statusCode, 401)
    assert.deepEqual(response.json(), { error: "principal is not allowed through http" })
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
