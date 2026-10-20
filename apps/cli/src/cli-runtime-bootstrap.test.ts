import { strict as assert } from "node:assert"
import { mkdtempSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
import { parseArgs } from "./cli-options.js"
import { CloudClientAuthError } from "./cloud-client-http.js"
import type { BootstrapState, CliOptions } from "./cli-types.js"
import type { CharioxPreferences } from "./preferences.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import {
  bootstrapCliRuntime,
  buildDetachedBootstrap,
  type CliRuntimeBootstrapDeps,
} from "./cli-runtime-bootstrap.js"

test("bootstrapCliRuntime falls back to detached waiting room when the default kernel is unreachable", async () => {
  const calls: string[] = []
  const options = cliOptions()
  const deps = createDeps({
    parseArgs: () => options,
    isNoArgDefaultKernelLaunch: () => true,
    isKernelEndpointReachable: async () => false,
    bootstrapAttachedSession: async () => {
      calls.push("bootstrapAttachedSession")
      throw new Error("should not attach")
    },
  })

  const result = await bootstrapCliRuntime({ argv: [], cwd: "/repo" }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(result.bootstrap.binding, null)
  assert.equal(result.bootstrap.options.detached, true)
  assert.equal(result.bootstrap.themeRegistry, DEFAULT_THEME_REGISTRY)
  assert.deepEqual(calls, [])
})

test("bootstrapCliRuntime attaches and resizes attached sessions", async () => {
  const calls: string[] = []
  const options = cliOptions({ kernelUrl: "ws://kernel.example/session" })
  const client = fakeClient()
  const deps = createDeps({
    parseArgs: () => options,
    createClient: () => client,
    bootstrapAttachedSession: async (attachedClient, attachedOptions, workspace, worktree, preferences) => {
      calls.push(`attach:${workspace}:${worktree}:${attachedOptions.clientId}:${attachedClient === client}`)
      return attachedBootstrap(attachedClient, attachedOptions, preferences)
    },
    maybeResize: async (resizeClient, sessionId) => {
      calls.push(`resize:${sessionId}:${resizeClient === client}`)
    },
  })

  const result = await bootstrapCliRuntime({ argv: ["--kernel-url", "ws://kernel.example/session"], cwd: "/repo" }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(result.kernelEndpoint, "ws://kernel.example/session")
  assert.equal(result.bootstrap.binding?.session.id, "session-1")
  assert.equal(result.bootstrap.themeRegistry, DEFAULT_THEME_REGISTRY)
  assert.deepEqual(calls, [
    "attach:/repo:/repo:cli-1:true",
    "resize:session-1:true",
  ])
})

test("MP-08 / MP-10: advisory resize does not delay the attached transcript", async () => {
  let finish!: () => void
  const pending = new Promise<void>(resolve => { finish = resolve })
  const client = fakeClient()
  const options = cliOptions({ kernelUrl: "ws://kernel.example/session" })
  const deps = createDeps({ parseArgs: () => options, createClient: () => client,
    bootstrapAttachedSession: async (c, o, _workspace, _worktree, prefs) => attachedBootstrap(c, o, prefs),
    maybeResize: () => pending,
  })
  const mounting = bootstrapCliRuntime({ argv: ["--session", "session-1"], cwd: "/repo" }, deps)
  const result = await Promise.race([mounting, new Promise<"blocked">(resolve => setTimeout(() => resolve("blocked"), 20))])
  finish()
  await mounting
  assert.notEqual(result, "blocked")
})

test("remote bootstrap passes the original issuing kernel route to the shared client", async () => {
  const issuer = { endpoint: "ws://127.0.0.1:49911/kernel", daemonId: "account-kernel" }
  const options = cliOptions({ relayUrl: "wss://relay.example", relayToken: "synthetic", targetDaemonId: "managed-kernel", relayTokenIssuer: issuer })
  let captured: unknown
  const client = fakeClient()
  const deps = createDeps({
    parseArgs: () => options,
    createClient: (endpoint, relayOptions) => {
      assert.equal(endpoint, options.relayUrl)
      captured = relayOptions?.relayAuthorizationIssuer
      return client
    },
  })
  await bootstrapCliRuntime({ argv: [], cwd: "/repo" }, deps)
  assert.deepEqual(captured, issuer)
})

test("bootstrapCliRuntime deletes a requested session without attaching", async () => {
  const calls: string[] = []
  const client = fakeClient()
  client.close = async () => {
    calls.push("close")
  }
  const options = cliOptions({
    deleteSessionRef: "old-session",
    workspace: "/workspace",
  })
  const deps = createDeps({
    parseArgs: () => options,
    createClient: () => client,
    deleteSessionByRef: async (_client, sessionRef, workspace) => {
      calls.push(`delete:${sessionRef}:${workspace}`)
    },
    bootstrapAttachedSession: async () => {
      calls.push("bootstrapAttachedSession")
      throw new Error("should not attach")
    },
    maybeResize: async () => {
      calls.push("resize")
    },
  })

  const result = await bootstrapCliRuntime({ argv: ["--delete-session", "old-session"], cwd: "/repo" }, deps)

  assert.equal(result.kind, "deleted_session")
  assert.equal(result.workspace, "/workspace")
  assert.deepEqual(calls, ["delete:old-session:/workspace", "close"])
})

test("terminal pairing bootstrap replaces the legacy token with one bound to the stored CLI key", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-bootstrap-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  const tokenPayload = Buffer.from(JSON.stringify({
    public_key_thumbprint: identity.publicKeyThumbprint, sub: "terminal-1", allowed_targets: ["home-1"], exp: Math.floor(Date.now()/1000)+300,
  })).toString("base64url")
  const boundRelayToken = `eyJhbGciOiJub25lIn0.${tokenPayload}.signature`
  const capturedRequests: unknown[] = []
  const createOptions: Array<{ relayAuthToken?: string; relayIdentity?: unknown }> = []
  let closedInitialClient = false
  const initialClient = fakeClient()
  initialClient.send = async <TResponse>(request: unknown) => {
    capturedRequests.push(request)
    return {
      TerminalPairingLinkJoined: {
        terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
        pairing: {
          intent: "client",
          subject_id: "terminal-1",
          relay_url: "wss://relay.example",
          target_daemon_id: "home-1",
          public_key_thumbprint: identity.publicKeyThumbprint,
          paired_at_ms: 1,
        },
        relay_token: boundRelayToken,
      },
    } as TResponse
  }
  initialClient.close = async () => { closedInitialClient = true }
  const attachedClient = fakeClient()
  attachedClient.startRelayAuthRenewal = expiry => { assert.ok(expiry > Date.now()) }
  const options = cliOptions({
    clientId: "terminal-1",
    relayUrl: "wss://relay.example",
    relayToken: "bootstrap-token",
    targetDaemonId: "home-1",
  })
  const deps = createDeps({
    parseArgs: () => options,
    getRelayIdentity: () => identity,
    createClient: (_endpoint, relayOptions) => {
      createOptions.push(relayOptions ?? {})
      return createOptions.length === 1 ? initialClient : attachedClient
    },
    bootstrapAttachedSession: async (client, attachedOptions, workspace, worktree, preferences) => {
      assert.equal(client, attachedClient)
      return attachedBootstrap(client, attachedOptions, preferences)
    },
  })

  const result = await bootstrapCliRuntime({
    argv: [terminalLink("bootstrap-token")],
    cwd: "/repo",
  }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(closedInitialClient, true)
  assert.deepEqual(createOptions.map((entry) => entry.relayAuthToken), ["bootstrap-token", boundRelayToken])
  assert.equal(createOptions[0]?.relayIdentity, identity)
  assert.equal(createOptions[1]?.relayIdentity, identity)
  const join = capturedRequests[0] as { JoinTerminalPairingLink: { public_key_thumbprint: string } }
  assert.equal(join.JoinTerminalPairingLink.public_key_thumbprint, identity.publicKeyThumbprint)
})

test("enrolled Cloud pairing obtains receiver bootstrap authority before the key-bound join", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-bootstrap-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  const tokenPayload = Buffer.from(JSON.stringify({
    public_key_thumbprint: identity.publicKeyThumbprint, sub: "terminal-1", allowed_targets: ["home-1"], exp: Math.floor(Date.now()/1000)+300,
  })).toString("base64url")
  const boundRelayToken = `eyJhbGciOiJub25lIn0.${tokenPayload}.signature`
  const capturedRequests: unknown[] = []
  const createOptions: Array<{ relayAuthToken?: string; relayIdentity?: unknown }> = []
  let closedInitialClient = false
  const initialClient = fakeClient()
  initialClient.send = async <TResponse>(request: unknown) => {
    capturedRequests.push(request)
    return {
      TerminalPairingLinkJoined: {
        terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
        pairing: {
          intent: "client",
          subject_id: "terminal-1",
          relay_url: "wss://relay.example",
          target_daemon_id: "home-1",
          public_key_thumbprint: identity.publicKeyThumbprint,
          paired_at_ms: 1,
        },
        relay_token: boundRelayToken,
      },
    } as TResponse
  }
  initialClient.close = async () => { closedInitialClient = true }
  const attachedClient = fakeClient()
  attachedClient.startRelayAuthRenewal = expiry => { assert.ok(expiry > Date.now()) }
  const options = cliOptions({
    clientId: "terminal-1",
    relayUrl: "wss://relay.example",
    relayToken: "cloud-client-token-required",
    targetDaemonId: "home-1",
  })
  const deps = createDeps({
    parseArgs: () => options,
    getRelayIdentity: () => identity,
    createClient: (_endpoint, relayOptions) => {
      createOptions.push(relayOptions ?? {})
      return createOptions.length === 1 ? initialClient : attachedClient
    },
    bootstrapAttachedSession: async (client, attachedOptions, workspace, worktree, preferences) => {
      assert.equal(client, attachedClient)
      return attachedBootstrap(client, attachedOptions, preferences)
    },
  })

  let resolved = 0
  Object.assign(deps, { resolvePairingBootstrapToken: async (relayUrl: string, kernelId: string, suppliedIdentity: unknown) => {
    assert.equal(relayUrl, "wss://relay.example")
    assert.equal(kernelId, "home-1")
    assert.equal(suppliedIdentity, identity)
    resolved++
    return "bootstrap-token"
  } })
  const result = await bootstrapCliRuntime({
    argv: [terminalLink("cloud-client-token-required")],
    cwd: "/repo",
  }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(resolved, 1)
  assert.equal(closedInitialClient, true)
  assert.deepEqual(createOptions.map((entry) => entry.relayAuthToken), ["bootstrap-token", boundRelayToken])
  assert.equal(createOptions[0]?.relayIdentity, identity)
  assert.equal(createOptions[1]?.relayIdentity, identity)
  const join = capturedRequests[0] as { JoinTerminalPairingLink: { terminal_id: string; public_key_thumbprint: string } }
  assert.equal(join.JoinTerminalPairingLink.public_key_thumbprint, identity.publicKeyThumbprint)
  // MP-08 / MP-10 / MP-11: link labels cannot replace the receiving CLIENT.
  assert.equal(join.JoinTerminalPairingLink.terminal_id, `cli:${identity.publicKeyThumbprint}`)
})

// MP-08 / MP-10 / MP-11: explicit transport credentials cannot change the
// invitation's receiving authority. Cover both sources and both flag orders.
for (const source of ["argv", "environment"] as const) {
  for (const tokenFirst of [false, true]) {
    test(`explicit Cloud CLIENT ${source} token ${tokenFirst ? "before" : "after"} the link retains receiving identity`, async t => {
      const root = mkdtempSync(path.join(os.tmpdir(), "chariox-explicit-client-"))
      t.after(() => rmSync(root, { recursive: true, force: true }))
      const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
      const receivingId = `cli:${identity.publicKeyThumbprint}`
      const token = `header.${Buffer.from(JSON.stringify({ sub: receivingId, public_key_thumbprint: identity.publicKeyThumbprint })).toString("base64url")}.synthetic`
      const envName = "CHARIOX_TEST_EXPLICIT_PAIRING_CLIENT"
      const previous = process.env[envName]
      process.env[envName] = token
      t.after(() => { if (previous === undefined) delete process.env[envName]; else process.env[envName] = previous })
      const link = terminalLink("cloud-client-token-required")
      const tokenArgs = source === "argv" ? ["--relay-token", token] : ["--relay-token-env", envName]
      const linkArgs = ["--terminal-pairing-link", link]
      const argv = tokenFirst ? [...tokenArgs, ...linkArgs] : [...linkArgs, ...tokenArgs]
      const initial = fakeClient(), attached = fakeClient()
      let joins = 0, creations = 0
      initial.send = async <T>(request: unknown) => {
        const join = (request as { JoinTerminalPairingLink: { terminal_id: string; public_key_thumbprint: string } }).JoinTerminalPairingLink
        // The enrolled-kernel join binds this request to the authenticated CLIENT.
        assert.equal(join.terminal_id, receivingId, "join identity must match the authenticated receiving CLIENT")
        assert.equal(join.public_key_thumbprint, identity.publicKeyThumbprint)
        joins++
        return { TerminalPairingLinkJoined: {
          terminal: { terminal_id: receivingId, terminal_type: "cli", paired_at_ms: 1, revoked: false },
          pairing: { intent: "client", subject_id: receivingId, relay_url: "wss://relay.example", target_daemon_id: "home-1", public_key_thumbprint: identity.publicKeyThumbprint, paired_at_ms: 1 },
          kernel_pairing: true,
        } } as T
      }
      const result = await bootstrapCliRuntime({ argv, cwd: "/repo" }, createDeps({
        parseArgs,
        getRelayIdentity: () => identity,
        resolvePairingBootstrapToken: async () => { throw new Error("explicit bootstrap token must be used without another login") },
        createClient: (_endpoint, options) => {
          assert.equal(options?.relayAuthToken, token)
          assert.equal(options?.relayIdentity, identity)
          return ++creations === 1 ? initial : attached
        },
      }))
      assert.equal(result.kind, "ready")
      assert.equal(joins, 1)
      assert.equal(creations, 2)
      if (result.kind === "ready") assert.equal(result.bootstrap.options.clientId, receivingId)
    })
  }
}

// MP-08 / MP-10 / MP-11: the same overrides on an operator invitation retain
// its own terminal label rather than adopting Cloud receiving authority.
for (const tokenFirst of [false, true]) {
  test(`explicit self-hosted token ${tokenFirst ? "before" : "after"} the link preserves invitation identity`, async t => {
    const root = mkdtempSync(path.join(os.tmpdir(), "chariox-explicit-selfhost-"))
    t.after(() => rmSync(root, { recursive: true, force: true }))
    const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
    const linkArgs = ["--terminal-pairing-link", terminalLink("operator-client-token-required")]
    const tokenArgs = ["--relay-token", "synthetic-operator-token"]
    const initial = fakeClient(), attached = fakeClient()
    let creations = 0
    initial.send = async <T>(request: unknown) => {
      assert.equal((request as { JoinTerminalPairingLink: { terminal_id: string } }).JoinTerminalPairingLink.terminal_id, "invitation-label")
      return { TerminalPairingLinkJoined: {
        terminal: { terminal_id: "invitation-label", terminal_type: "cli", paired_at_ms: 1, revoked: false },
        pairing: { intent: "client", subject_id: "invitation-label", relay_url: "wss://relay.example", target_daemon_id: "home-1", public_key_thumbprint: identity.publicKeyThumbprint, paired_at_ms: 1 },
        kernel_pairing: true,
      } } as T
    }
    const result = await bootstrapCliRuntime({ argv: tokenFirst ? [...tokenArgs, ...linkArgs] : [...linkArgs, ...tokenArgs], cwd: "/repo" }, createDeps({
      parseArgs, getRelayIdentity: () => identity,
      resolvePairingBootstrapToken: async () => { throw new Error("self-hosted pairing cannot request Cloud authority") },
      createClient: (_endpoint, options) => {
        assert.equal(options?.relayAuthToken, "synthetic-operator-token")
        return ++creations === 1 ? initial : attached
      },
    }))
    assert.equal(result.kind, "ready")
    assert.equal(creations, 2)
  })
}

function terminalLink(relayToken: string, relayUrl = "wss://relay.example", targetDaemonId = "home-1") {
  return `chariox-terminal-pair-v1.${Buffer.from(JSON.stringify({ relay_url: relayUrl, relay_token: relayToken, target_daemon_id: targetDaemonId, terminal_id: "invitation-label" })).toString("base64url")}`
}

test("terminal pairing closes bootstrap transport and rejects a legacy unbound token", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-bootstrap-legacy-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  let closeCount = 0
  let createCount = 0
  const initialClient = fakeClient()
  initialClient.send = async <TResponse>() => {
    return {
      TerminalPairingLinkJoined: {
        terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
        pairing: {
          intent: "client",
          subject_id: "terminal-1",
          relay_url: "wss://relay.example",
          target_daemon_id: "home-1",
          public_key_thumbprint: identity.publicKeyThumbprint,
          paired_at_ms: 1,
        },
      },
    } as TResponse
  }
  initialClient.close = async () => { closeCount += 1 }
  const deps = createDeps({
    parseArgs: () => cliOptions({
      clientId: "terminal-1",
      relayUrl: "wss://relay.example",
      relayToken: "bootstrap-token",
      targetDaemonId: "home-1",
    }),
    getRelayIdentity: () => identity,
    createClient: () => {
      createCount += 1
      return initialClient
    },
  })

  await assert.rejects(
    bootstrapCliRuntime({ argv: [terminalLink("bootstrap-token")], cwd: "/repo" }, deps),
    /requires a kernel-issued admission or a fresh bound relay token/,
  )
  assert.equal(createCount, 1)
  assert.equal(closeCount, 1)
})

test("MP-08 a relay-addressed kernel elsewhere keeps the relay client", async () => {
  const relayClient = fakeClient()
  const deps = createDeps({
    parseArgs: () => cliOptions({ relayUrl: "wss://relay.example", relayToken: "token", targetDaemonAlias: "home" }),
    createClient: () => relayClient,
    bootstrapAttachedSession: async (client, attachedOptions, _workspace, _worktree, preferences) => {
      assert.equal(client, relayClient)
      return attachedBootstrap(client, attachedOptions, preferences)
    },
  })

  const result = await bootstrapCliRuntime({ argv: [], cwd: "/repo" }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(result.kernelEndpoint, "wss://relay.example")
})

test("buildDetachedBootstrap creates a waiting-room bootstrap shell", () => {
  const client = fakeClient()
  const options = cliOptions({ detached: true })
  const preferences: CharioxPreferences = {}

  const bootstrap = buildDetachedBootstrap(client, options, preferences)

  assert.equal(bootstrap.client, client)
  assert.equal(bootstrap.binding, null)
  assert.deepEqual(bootstrap.sessions, [])
  assert.equal(bootstrap.options, options)
  assert.equal(bootstrap.preferences, preferences)
})

// MP-08 / MP-10 / MP-11: later Cloud pivots and kernel-issued grants must use
// the same receiving CLIENT as device login, including a first login in the TUI.
for (const launch of ["explicit", "unreachable", "lost-during-attach"] as const) {
  test(`detached ${launch} launch retains the receiving CLIENT identity for later attachment`, async t => {
    const root = mkdtempSync(path.join(os.tmpdir(), "chariox-detached-client-"))
    t.after(() => rmSync(root, {recursive: true, force: true}))
    const store = createCliRelayIdentityStore(path.join(root, "relay", "cli-identity-v1.json"))
    const options = cliOptions({detached: launch === "explicit"})
    const result = await bootstrapCliRuntime({argv: launch === "explicit" ? ["--detached"] : [], cwd: "/repo"}, createDeps({
      parseArgs: () => options,
      getRelayIdentity: create => create ? store.getOrCreate() : store.load(),
      isNoArgDefaultKernelLaunch: () => launch !== "explicit",
      isKernelEndpointReachable: async () => launch !== "unreachable",
      bootstrapAttachedSession: async () => {throw new Error("kernel unavailable")},
      isKernelEndpointUnavailableError: () => true,
    }))
    assert.equal(result.kind, "ready")
    if (result.kind !== "ready") throw new Error("expected ready")
    assert.equal(result.bootstrap.binding, null)
    assert.equal(result.bootstrap.options.clientId, `cli:${store.getOrCreate().publicKeyThumbprint}`)
    assert.equal(store.load()?.publicKeyThumbprint, store.getOrCreate().publicKeyThumbprint)
  })
}

function createDeps(overrides: Partial<CliRuntimeBootstrapDeps> = {}): CliRuntimeBootstrapDeps {
  const client = fakeClient()
  return {
    parseArgs: () => cliOptions(),
    loadPreferences: async () => ({}),
    applyProviderPreferenceDefaults: (options) => options,
    defaultKernelEndpoint: () => "ws://127.0.0.1:43118/kernel",
    createClient: () => client,
    getRelayIdentity: () => null,
    inferWorkspaceTargetsFromLaunchDirectory: async (cwd) => ({
      workspace: cwd,
      worktree: cwd,
    }),
    clearWaitingRoomWorktreeInventory: () => {},
    loadThemeRegistry: async () => DEFAULT_THEME_REGISTRY,
    deleteSessionByRef: async () => {},
    isNoArgDefaultKernelLaunch: () => false,
    isKernelEndpointReachable: async () => true,
    isKernelEndpointUnavailableError: () => false,
    bootstrapAttachedSession: async (attachedClient, attachedOptions, _workspace, _worktree, preferences) =>
      attachedBootstrap(attachedClient, attachedOptions, preferences),
    maybeResize: async () => {},
    ...overrides,
  }
}

function cliOptions(overrides: Partial<CliOptions> = {}): CliOptions {
  return {
    clientId: "cli-1",
    provider: "opencode",
    model: "default",
    accountProfile: "default",
    effort: "",
    ...overrides,
  }
}

function fakeClient(): LocalIpcClient {
  return new LocalIpcClient("/unused-kernel.sock")
}

function attachedBootstrap(
  client: LocalIpcClient,
  options: CliOptions,
  preferences: CharioxPreferences,
): BootstrapState {
  return {
    client,
    binding: {
      session: { id: "session-1" },
      attachment: { id: "attachment-1" },
      providerRun: null,
      createdSession: true,
      historyEntries: [],
      promptHistoryEntries: [],
      nextHistoryCursor: null,
    },
    sessions: [],
    providerCatalog: {},
    providerCommandCatalogs: {},
    options,
    preferences,
  } as unknown as BootstrapState
}

test("Cloud-free pairing keeps operator transport while the kernel admits the persisted terminal key", async t => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-selfhost-bootstrap-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  const initial = fakeClient(), attached = fakeClient()
  const options = cliOptions({ clientId:"terminal-one", relayUrl:"ws://relay.local", relayToken:"synthetic-operator-token", targetDaemonId:"home" })
  const captured: Array<{relayAuthToken?:string;relayIdentity?:unknown}> = []
  initial.send = async <T>() => ({ TerminalPairingLinkJoined: {
    terminal: { terminal_id:"terminal-one",terminal_type:"cli",paired_at_ms:1,revoked:false },
    pairing: { intent:"client", subject_id:"terminal-one",relay_url:"ws://relay.local",target_daemon_id:"home",public_key_thumbprint:identity.publicKeyThumbprint,paired_at_ms:1 },
    kernel_pairing:true,
  } }) as T
  const deps = createDeps({ parseArgs:()=>options, getRelayIdentity:()=>identity,
    createClient:(_endpoint,relayOptions)=>{captured.push(relayOptions ?? {});return captured.length===1?initial:attached},
    bootstrapAttachedSession:async(client,opts,_workspace,_worktree,prefs)=>attachedBootstrap(client,opts,prefs),
  })
  const result = await bootstrapCliRuntime({argv:[terminalLink("synthetic-operator-token", "ws://relay.local", "home")],cwd:"/repo"},deps)
  assert.equal(result.kind,"ready")
  assert.equal(captured.length,2)
  assert.equal(captured[1]?.relayAuthToken,"synthetic-operator-token")
  assert.equal(captured[1]?.relayIdentity,identity)
  assert.equal(createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).load()?.publicKeyThumbprint,identity.publicKeyThumbprint)
})

// MP-08 / MP-11: exercise the final production bootstrap client, commands and events.
for (const source of ["automatic", "argv", "environment"] as const) {
test(`Cloud pairing ${source} bootstrap keeps commands and events across repeated grant expiries`, async t => {
  t.mock.timers.enable({apis:["Date","setTimeout","setInterval"],now:Date.now()})
  const server = new WebSocketServer({host:"127.0.0.1",port:0})
  await new Promise<void>(resolve => server.once("listening",resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const relayUrl = `ws://127.0.0.1:${address.port}`
  const root=mkdtempSync(path.join(os.tmpdir(),"chariox-kauth-renewal-"))
  t.after(()=>rmSync(root,{recursive:true,force:true}))
  const identity=createCliRelayIdentityStore(`${root}/terminal.json`).getOrCreate()
  const daemon=createCliRelayIdentityStore(`${root}/daemon.json`).getOrCreate()
  const timers = new Set<ReturnType<typeof setTimeout>>()
  const streams = new Set<ReturnType<typeof setInterval>>()
  let renewals=0, events=0, subscriptions=0, receivingChecks=0, receivingDenials=0, receivingRevoked=false
  let subscribedSocket: unknown, reauthorizations=0
  const waitForProgress=async (ready:()=>boolean,reason:string)=>{
    const deadline=performance.now()+10_000
    while(!ready()){
      assert.ok(performance.now()<deadline,reason)
      await new Promise<void>(resolve=>setImmediate(resolve))
    }
  }
  const grantLifetimeMs=2_000
  const grant = () => {
    const expiresAtMs=Date.now()+grantLifetimeMs
    const payload=Buffer.from(JSON.stringify({sub:"paired-subject",public_key_thumbprint:identity.publicKeyThumbprint,allowed_targets:["home"],expires_at_ms:expiresAtMs})).toString("base64url")
    return {token:`chariox-scoped-v1.${payload}.synthetic`,expiresAtMs}
  }
  server.on("connection",socket => {
    let expiryTimer: ReturnType<typeof setTimeout> | undefined
    socket.on("message",raw => {
      const frame=JSON.parse(String(raw))
      if(frame.kind==="client_connect") {
        if(socket===subscribedSocket)reauthorizations++
        if(expiryTimer) {clearTimeout(expiryTimer);timers.delete(expiryTimer)}
        const expiry=frame.auth_token==="bootstrap" ? Date.now()+5_000 : JSON.parse(Buffer.from(frame.auth_token.split(".")[1],"base64url").toString()).expires_at_ms
        expiryTimer=setTimeout(()=>socket.close(),Math.max(0,expiry-Date.now()));timers.add(expiryTimer)
        socket.send(JSON.stringify({kind:"client_connected",target:frame.target,daemon_public_key:daemon.publicKeyBase64}))
      }
      if(frame.kind==="client_request") {
        const request=JSON.parse(daemon.decrypt(frame.encrypted_request)).request
        let response: unknown={accepted:true}
        if(request.JoinTerminalPairingLink) {
          const g=grant()
          response={TerminalPairingLinkJoined:{terminal:{terminal_id:"paired-subject",terminal_type:"cli",paired_at_ms:1,revoked:false},pairing:{intent:"client",subject_id:"paired-subject",relay_url:relayUrl,target_daemon_id:"home",public_key_thumbprint:identity.publicKeyThumbprint,paired_at_ms:1},kernel_pairing:false,relay_token:g.token}}
        }
        if(request.IssueCloudRelayClientToken) {
          assert.deepEqual(request.IssueCloudRelayClientToken,{target_daemon_alias:"home",client_id:`cli:${identity.publicKeyThumbprint}`,session_id:null,public_key_thumbprint:identity.publicKeyThumbprint})
          if(receivingRevoked) {
            receivingDenials++
            socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,encrypted_response:daemon.encrypt(frame.encrypted_request.sender_public_key,JSON.stringify({Error:{message:"receiving client revoked",code:"authorization_denied",retryable:false}})),error:null}))
            return
          }
          renewals++
          const g=grant()
          response={CloudRelayClientTokenIssued:{profile:{api_url:"http://cloud.test",email:"owner@example.test",account_id:"account",user_id:"owner",account_slug:"account",realm_id:"realm",relay_url:relayUrl,issuer_id:"fixture"},token:{relay_url:relayUrl,relay_token:g.token,token_expires_at:new Date(g.expiresAtMs).toISOString()}}}
        }
        socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,encrypted_response:daemon.encrypt(frame.encrypted_request.sender_public_key,JSON.stringify(response)),error:null}))
      }
      if(frame.kind==="client_subscribe") {
        subscribedSocket=socket
        subscriptions++
        socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,encrypted_response:daemon.encrypt(frame.client_public_key,"null"),error:null}))
        let id=0
        const timer=setInterval(()=>{if(socket.readyState===WebSocket.OPEN)socket.send(JSON.stringify({kind:"client_event",subscription_id:frame.subscription_id,event_id:++id,encrypted_event:daemon.encrypt(frame.client_public_key,JSON.stringify({event:"runtime_notices",notices:[{message:"continuing"}]}))}))},25)
        streams.add(timer);socket.once("close",()=>clearInterval(timer))
      }
    })
  })
  let finalClient: LocalIpcClient | undefined
  try {
    const link=terminalLink("cloud-client-token-required", relayUrl, "home")
    const envName="CHARIOX_TEST_RENEWAL_CLIENT"
    const previous=process.env[envName]
    process.env[envName]="bootstrap"
    t.after(()=>{if(previous===undefined)delete process.env[envName];else process.env[envName]=previous})
    const argv=["--terminal-pairing-link",link,...(source==="automatic"?[]:source==="argv"?["--relay-token","bootstrap"]:["--relay-token-env",envName])]
    const result=await bootstrapCliRuntime({argv,cwd:"/repo"},createDeps({parseArgs,getRelayIdentity:()=>identity,resolvePairingBootstrapToken:async()=>{receivingChecks++;if(source!=="automatic")throw new CloudClientAuthError("client_signed_out");if(receivingRevoked)throw new CloudClientAuthError("client_revoked");return "bootstrap"},createClient:(endpoint,opts)=>new LocalIpcClient(endpoint,opts)}))
    assert.equal(result.kind,"ready")
    if(result.kind!=="ready")throw new Error("expected ready")
    finalClient=result.bootstrap.client
    finalClient.onKernelEvent(event=>{if(event.event==="runtime_notices")events++})
    await finalClient.subscribeToKernelEvents("session","attachment")
    // Advance lease time only after the real socket has acknowledged renewal.
    // This verifies continuity without making CI scheduler pauses expire grants.
    for(let round=0;round<5;round++){
      const before=renewals, previousAuthorizations=reauthorizations
      t.mock.timers.tick(grantLifetimeMs-100)
      await waitForProgress(()=>renewals>before && reauthorizations>previousAuthorizations,"paired renewal must reauthorize the existing socket")
      assert.deepEqual(await finalClient.send({GetDaemonHealth:null}),{accepted:true})
      await new Promise<void>(resolve=>setImmediate(resolve))
    }
    await waitForProgress(()=>events>40,"renewed subscription must deliver events")
    assert.ok(renewals>=5,"final paired client renews repeatedly")
    // MP-08 / MP-10 / MP-11: kernel pivot renewal cannot outlive receiver CLIENT authority.
    if(source==="automatic")assert.ok(receivingChecks>=renewals+1,"every automatic kernel pivot renewal rechecks the independently enrolled receiving CLIENT")
    else assert.equal(receivingChecks,0,"explicit renewal needs no private receiving Cloud profile")
    assert.ok(events>40,"event lane remains active")
    assert.equal(subscriptions,1,"renewal preserves event subscription")
    receivingRevoked=true
    const issuedBeforeRevoke=renewals
    const checksBeforeRevoke=receivingChecks, denialsBeforeRevoke=receivingDenials
    t.mock.timers.tick(grantLifetimeMs-100)
    await waitForProgress(()=>source==="automatic" ? receivingChecks>checksBeforeRevoke : receivingDenials>denialsBeforeRevoke,"revoked receiving authority must reject renewal")
    await new Promise<void>(resolve=>setImmediate(resolve))
    t.mock.timers.tick(grantLifetimeMs+100)
    await assert.rejects(finalClient.send({GetDaemonHealth:null}), /revoked|authorization|client|authority has expired/i)
    assert.equal(renewals,issuedBeforeRevoke,"revoked receiving CLIENT cannot get another kernel pivot")
    if(source!=="automatic")assert.ok(receivingDenials>0,"kernel/Cloud must reject the revoked receiving CLIENT")
  } finally {
    await finalClient?.close()
    for(const timer of timers)clearTimeout(timer)
    for(const timer of streams)clearInterval(timer)
    for(const socket of server.clients)socket.terminate()
    await new Promise<void>(resolve=>server.close(()=>resolve()))
  }
})
}
