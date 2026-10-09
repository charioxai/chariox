import { strict as assert } from "node:assert"
import { mkdtempSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
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
    argv: ["chariox-terminal-pair-v1.fixture"],
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
    argv: ["chariox-terminal-pair-v1.fixture"],
    cwd: "/repo",
  }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(resolved, 1)
  assert.equal(closedInitialClient, true)
  assert.deepEqual(createOptions.map((entry) => entry.relayAuthToken), ["bootstrap-token", boundRelayToken])
  assert.equal(createOptions[0]?.relayIdentity, identity)
  assert.equal(createOptions[1]?.relayIdentity, identity)
  const join = capturedRequests[0] as { JoinTerminalPairingLink: { public_key_thumbprint: string } }
  assert.equal(join.JoinTerminalPairingLink.public_key_thumbprint, identity.publicKeyThumbprint)
})

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
    bootstrapCliRuntime({ argv: ["chariox-terminal-pair-v1.fixture"], cwd: "/repo" }, deps),
    /requires a kernel-issued admission or a fresh bound relay token/,
  )
  assert.equal(createCount, 1)
  assert.equal(closeCount, 1)
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
  const result = await bootstrapCliRuntime({argv:["chariox-terminal-pair-v1.fixture"],cwd:"/repo"},deps)
  assert.equal(result.kind,"ready")
  assert.equal(captured.length,2)
  assert.equal(captured[1]?.relayAuthToken,"synthetic-operator-token")
  assert.equal(captured[1]?.relayIdentity,identity)
  assert.equal(createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).load()?.publicKeyThumbprint,identity.publicKeyThumbprint)
})

// MP-08 / MP-11: exercise the final production bootstrap client, commands and events.
test("Cloud pairing bootstrap keeps commands and events across repeated grant expiries", async t => {
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
  let renewals=0, events=0, subscriptions=0
  const grant = () => {
    const expiresAtMs=Date.now()+400
    const payload=Buffer.from(JSON.stringify({sub:"paired-subject",public_key_thumbprint:identity.publicKeyThumbprint,allowed_targets:["home"],expires_at_ms:expiresAtMs})).toString("base64url")
    return {token:`chariox-scoped-v1.${payload}.synthetic`,expiresAtMs}
  }
  server.on("connection",socket => {
    let expiryTimer: ReturnType<typeof setTimeout> | undefined
    socket.on("message",raw => {
      const frame=JSON.parse(String(raw))
      if(frame.kind==="client_connect") {
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
          response={TerminalPairingLinkJoined:{terminal:{terminal_id:"paired-subject",terminal_type:"cli",paired_at_ms:1,revoked:false},pairing:{intent:"client",subject_id:"paired-subject",relay_url:relayUrl,target_daemon_id:"home",public_key_thumbprint:identity.publicKeyThumbprint,paired_at_ms:1},relay_token:g.token}}
        }
        if(request.IssueCloudRelayClientToken) {
          assert.deepEqual(request.IssueCloudRelayClientToken,{target_daemon_alias:"home",client_id:"terminal-1",session_id:null,public_key_thumbprint:identity.publicKeyThumbprint})
          renewals++
          const g=grant()
          response={CloudRelayClientTokenIssued:{profile:{api_url:"http://cloud.test",email:"owner@example.test",account_id:"account",user_id:"owner",account_slug:"account",realm_id:"realm",relay_url:relayUrl,issuer_id:"fixture"},token:{relay_url:relayUrl,relay_token:g.token,token_expires_at:new Date(g.expiresAtMs).toISOString()}}}
        }
        socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,encrypted_response:daemon.encrypt(frame.encrypted_request.sender_public_key,JSON.stringify(response)),error:null}))
      }
      if(frame.kind==="client_subscribe") {
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
    const result=await bootstrapCliRuntime({argv:["chariox-terminal-pair-v1.fixture"],cwd:"/repo"},createDeps({parseArgs:()=>cliOptions({clientId:"terminal-1",relayUrl,relayToken:"cloud-client-token-required",targetDaemonId:"home"}),getRelayIdentity:()=>identity,resolvePairingBootstrapToken:async()=>"bootstrap",createClient:(endpoint,opts)=>new LocalIpcClient(endpoint,opts)}))
    assert.equal(result.kind,"ready")
    if(result.kind!=="ready")throw new Error("expected ready")
    finalClient=result.bootstrap.client
    finalClient.onKernelEvent(event=>{if(event.event==="runtime_notices")events++})
    await finalClient.subscribeToKernelEvents("session","attachment")
    const deadline=Date.now()+2_000
    while(Date.now()<deadline){assert.deepEqual(await finalClient.send({GetDaemonHealth:null}),{accepted:true});await new Promise(resolve=>setTimeout(resolve,25))}
    assert.ok(renewals>=5,"final paired client renews repeatedly")
    assert.ok(events>40,"event lane remains active")
    assert.equal(subscriptions,1,"renewal preserves event subscription")
  } finally {
    await finalClient?.close()
    for(const timer of timers)clearTimeout(timer)
    for(const timer of streams)clearInterval(timer)
    for(const socket of server.clients)socket.terminate()
    await new Promise<void>(resolve=>server.close(()=>resolve()))
  }
})
