import {
  acquireAgentAppReplica,
  appendCloudPublicationDeploymentLogs,
  assert,
  baseConfig,
  buildServer,
  clearAgentAppEffectStoresForTests,
  clearAgentAppReplicaPoolsForTests,
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  createServer,
  findWorkflowRunByInvocationRequestId,
  firstSetCookieValue,
  invokePublicationInput,
  join,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  mkdir,
  mkdtemp,
  promptFromInvocationInput,
  providerCatalogResponse,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
  publicationForAgentAppInvocation,
  publicationInvocationEnvelope,
  publishedHttpConfig,
  readFile,
  registerCloudPublicationDeploymentBackend,
  releaseAgentAppReplicaInvocation,
  rememberAgentAppInvocationRoute,
  rm,
  setOptionalEnv,
  test,
  tmpdir,
  visibleWorkflowRun,
  waitForCondition,
  writeFile,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"

test("agent app gateway serves packaged app assets", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-assets-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  await writeFile(join(root, "app", "styles.css"), "main { color: red; }")
  const { app } = buildServer({
    ...baseConfig,
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [],
    },
  })

  try {
    const index = await app.inject({ method: "GET", url: "/" })
    assert.equal(index.statusCode, 200)
    assert.match(index.headers["content-type"] as string, /text\/html/)
    assert.equal(index.body, "<!doctype html><main>shop</main>")

    const styles = await app.inject({ method: "GET", url: "/styles.css" })
    assert.equal(styles.statusCode, 200)
    assert.match(styles.headers["content-type"] as string, /text\/css/)
    assert.equal(styles.body, "main { color: red; }")

    const traversal = await app.inject({ method: "GET", url: "/../publication.json" })
    assert.notEqual(traversal.statusCode, 200)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app config validation rejects invalid launch config before serving", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-invalid-config-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  try {
    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-route",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        routes: [{ path: "add/*" }],
      },
    }), /route path must start with \//)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-action",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        routes: [{
          path: "/add/*",
          manipulation: { allowed_actions: ["missing-action"] },
        }],
      },
    }), /unknown action missing-action/)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-replicas",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        replicas: { count: 0 },
        routes: [{ path: "/add/*" }],
      },
    }), /replicas\.count/)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-missing-assets",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "missing", index: "index.html" },
        routes: [{ path: "/add/*" }],
      },
    }), /assets\.public_dir does not exist/)

    for (const url of [
      "http://169.254.169.254/latest/meta-data",
      "http://localhost:33119/action",
      "https://127.0.0.1:33119/action",
      "http://user:password@127.0.0.1:33119/action",
    ]) {
      assert.throws(() => buildServer({
        ...baseConfig,
        publication_id: "pub-unsafe-action-url",
        package_root: root,
        agent_app: {
          enabled: true,
          assets: { public_dir: "app", index: "index.html" },
          routes: [{ path: "/action", manipulation: { allowed_actions: ["unsafe"] } }],
          actions: { unsafe: { transport: { kind: "http", method: "POST", url } } },
        },
      }), /explicit loopback HTTP action-server URL/)
    }

    for (const host of ["*.example.com", "EXAMPLE.com", "127.0.0.1", "localhost", "example.com."]) {
      assert.throws(() => buildServer({
        ...baseConfig,
        publication_id: "pub-unsafe-network-host",
        package_root: root,
        agent_app: {
          enabled: true,
          assets: { public_dir: "app", index: "index.html" },
          routes: [],
          network: { destinations: [{ id: "integration:unsafe", host }] },
        },
      }), /exact canonical DNS host/)
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app wrapped route invokes workflow with path-tail prompt and streams viewer shell", async () => {
  let seenInput: unknown = null
  let seenProof: Record<string, unknown> | null = null
  const previousPort = process.env.PORT
  process.env.PORT = "34567"
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-route-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    trace_exposure: { nodes: { "node-1": ["output_summary", "assistant_messages", "thinking", "tool_use"] } },
    trace_context: {
      nodes: {
        "node-1": {
          node_id: "node-1",
          node_label: "Checkout Agent",
          agent_id: "agent-1",
          agent_alias: "shopper",
        },
      },
    },
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          scope: "session",
          allowed_actions: ["cart.add"],
        },
      }],
      actions: {
        "cart.add": {
          input_schema: {
            type: "object",
            required: ["sku"],
            properties: { sku: { type: "string" } },
          },
          transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/add" },
        },
        "cart.admin": {
          transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/admin" },
        },
      },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      seenProof = invocation.caller.proof as Record<string, unknown>
      return {
        accepted: true,
        workflow_run: { id: "run-shopping", status: "Running" },
      }
    },
  })

  try {
    const response = await app.inject({
      method: "GET",
      url: "/add/1%20kg%20bananas",
      headers: { accept: "text/html" },
    })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/html/)
    assert.match(response.body, /class="publication-viewer has-traces has-composer"/)
    assert.match(response.body, /run-shopping/)
    assert.deepEqual(seenInput, { prompt: "1 kg bananas" })
    const proof = seenProof as Record<string, unknown> | null
    assert.deepEqual(Object.keys((proof?.agent_app_actions as Record<string, unknown>) ?? {}), ["cart.add"])
    assert.deepEqual(
      (proof?.agent_app_audit as Record<string, unknown> | undefined)?.url,
      "http://127.0.0.1:34567/.well-known/chariox/agent-app/audit-log",
    )
    const auditToken = (proof?.agent_app_audit as Record<string, unknown> | undefined)?.token
    assert.equal(typeof auditToken, "string")
    const auditResponse = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/audit-log",
      payload: {
        token: auditToken,
        entries: [{
          level: "info",
          message: "agent app action `cart.add` completed",
          metadata: { kind: "agent_app_action", action_id: "cart.add" },
        }],
      },
    })
    assert.equal(auditResponse.statusCode, 200)
    assert.deepEqual(JSON.parse(auditResponse.body), { accepted: true, appended: false })
  } finally {
    setOptionalEnv("PORT", previousPort)
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app final response effects overlay generated files for serve mode", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-overlay-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    trace_exposure: { nodes: { "node-1": ["output_summary", "assistant_messages", "thinking", "tool_use"] } },
    trace_context: {
      nodes: {
        "node-1": {
          node_id: "node-1",
          node_label: "Checkout Agent",
          agent_id: "agent-1",
          agent_alias: "shopper",
        },
      },
    },
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          scope: "session",
          allowed_paths: ["/generated/**"],
          protected_paths: ["/payments/**"],
          allowed_actions: ["cart.search", "cart.add", "cart.checkout"],
        },
      }],
      actions: {
        "cart.search": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/search" } },
        "cart.add": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/add" } },
        "cart.checkout": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/checkout" } },
      },
    },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-shopping",
        status: "Completed",
        workflow_id: "workflow-1",
        completed_by_node_run_id: "node-run-1",
        completed_at_ms: 1_700_000_000_000,
        node_runs: [{
          id: "node-run-1",
          node_id: "node-1",
          agent_id: "agent-1",
          status: "Completed",
          summary: "Prepared checkout from the shopping list.",
          completion: { summary: "Checkout ready with three products." },
          thinking_traces: [{ id: "thinking-1", message: "Map quantities to catalog SKUs before checkout.", timestamp_ms: 1_700_000_000_001 }],
          turn_envelope: {
            runtime_tool_calls: [{
              tool_name: "cart.checkout",
              arguments_json: "{\"items\":3}",
              result_json: "{\"ok\":true}",
              ok: true,
              timestamp_ms: 1_700_000_000_002,
            }],
          },
        }],
        messages: [{
          id: "message-1",
          source_node_run_id: "node-run-1",
          target_node_id: "node-1",
          message_type: "assistant",
          summary: "Added bananas, Coca-Cola, and chips to the basket.",
          handoff_payload: "",
          created_at_ms: 1_700_000_000_003,
        }],
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/checkout.html" },
            effects: {
              overlay: [{
                path: "/generated/checkout.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>custom banana checkout</main>",
              }],
            },
          }),
        },
      },
    }),
  })

  try {
    const invoke = await app.inject({
      method: "GET",
      url: "/add/1%20kg%20bananas",
      headers: { accept: "text/html" },
    })
    assert.equal(invoke.statusCode, 200)
    assert.match(invoke.body, /run-shopping/)
    assert.match(invoke.body, /initialTraces/)
    assert.match(invoke.body, /Checkout ready with three products/)
    assert.match(invoke.body, /cart.checkout ok/)
    assert.match(invoke.body, /frame\.src = publicationAppAssetUrl\(renderable\.src\)/)
    const cookie = firstSetCookieValue(invoke.headers["set-cookie"])
    assert.match(cookie, /chariox_agent_app_session=/)

    const checkout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie } })
    assert.equal(checkout.statusCode, 200)
    assert.match(checkout.headers["content-type"] as string, /text\/html/)
    assert.equal(checkout.body, "<!doctype html><main>custom banana checkout</main>")

    const unrelatedSession = await app.inject({ method: "GET", url: "/generated/checkout.html" })
    assert.equal(unrelatedSession.statusCode, 404)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app session overlays are isolated by browser session", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-session-overlay-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          scope: "session",
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      const prompt = (invocation.input as { prompt?: string }).prompt ?? "unknown"
      return {
        accepted: true,
        workflow_run: {
          id: `run-${prompt}`,
          status: "Completed",
          final_output: {
            message: JSON.stringify({
              kind: "response",
              response: { mode: "serve", entry: "/generated/checkout.html" },
              effects: {
                overlay: [{
                  path: "/generated/checkout.html",
                  mime_type: "text/html; charset=utf-8",
                  content: `<!doctype html><main>${prompt} checkout</main>`,
                }],
              },
            }),
          },
        },
      }
    },
  })

  try {
    const firstInvoke = await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html" } })
    assert.equal(firstInvoke.statusCode, 200)
    const firstCookie = firstSetCookieValue(firstInvoke.headers["set-cookie"])

    const secondInvoke = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html" } })
    assert.equal(secondInvoke.statusCode, 200)
    const secondCookie = firstSetCookieValue(secondInvoke.headers["set-cookie"])
    assert.notEqual(firstCookie, secondCookie)

    const firstCheckout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie: firstCookie } })
    assert.equal(firstCheckout.statusCode, 200)
    assert.equal(firstCheckout.body, "<!doctype html><main>bananas checkout</main>")

    const secondCheckout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie: secondCookie } })
    assert.equal(secondCheckout.statusCode, 200)
    assert.equal(secondCheckout.body, "<!doctype html><main>chips checkout</main>")
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})
