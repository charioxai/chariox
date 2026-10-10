import assert from "node:assert/strict"
import { mkdtemp, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import { createCloudDrillClient, loadCloudRelayDrillModules } from "./live-cloud-relay-drill-helpers.mjs"
import { createHostedCommandDeps, manualCloudDeviceLogin } from "./live-hosted-cloud-relay-drill-helpers.mjs"
import { assert as drillAssert, cleanupHostedCloudTerminal, connectSessionScopedCloudClient, installSendRetry, terminateChild, unwrap } from "./live-hosted-cloud-relay-drill-helpers.mjs"
import { runHostedMultiUserAssertions } from "./hosted-cloud-multi-user-scenarios.mjs"
import net from "node:net"
import { runHostedRemoteCliAssertions, runHostedRemoteCliPairingAssertions } from "./hosted-cloud-remote-cli-scenarios.mjs"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"
import { runRemoteTerminalLogin } from "./hosted-cloud-remote-terminal-login.mjs"
import { bootstrapCliRuntime, defaultCliRuntimeBootstrapDeps } from "../../dist/cli-runtime-bootstrap.js"

const modules = await loadCloudRelayDrillModules()

function mockClientLogin(t, calls) {
  let identity
  t.mock.method(globalThis, "fetch", async (url, init) => {
    const route = new URL(url).pathname
    const body = JSON.parse(init.body)
    calls.push(route)
    if (route === "/auth/device/start") {
      assert.equal(body.enrollmentKind, "CLIENT")
      assert.equal(body.clientId, `cli:${body.publicKeyThumbprint}`)
      assert.equal(body.machineId, undefined)
      identity = body
      return Response.json({deviceCode: "synthetic-device", userCode: "PUBLIC", verificationUrl: "http://127.0.0.1/activate", expiresAt: new Date(Date.now()+60_000).toISOString(), intervalSeconds: 1})
    }
    assert.equal(route, "/auth/device/poll")
    return Response.json({status: "approved", profile: {enrollmentKind: "CLIENT", publicKeyThumbprint: identity.publicKeyThumbprint, clientId: identity.clientId, email: "human@example.test", accountSlug: "human", accountId: "account", userId: "human", realmId: "realm", relayUrl: "wss://relay.test", issuerId: "issuer"}, cloudSessionToken: "synthetic-human-access", refreshCredential: "synthetic-refresh", cloudSessionExpiresAt: new Date(Date.now()+300_000).toISOString()})
  })
}

test("MP-08 / MP-10 / MP-11: hosted peer and third login use separate CLIENT stores and never kernel enrollment", async t => {
  const stateRoot = await mkdtemp(path.join(tmpdir(), "chariox-hosted-login-"))
  t.after(() => rm(stateRoot, {recursive: true, force: true}))
  const calls = [], terminals = []
  t.after(() => terminals.forEach(terminal => terminal.client.stop()))
  mockClientLogin(t, calls)
  for (const role of ["peer", "third"]) {
    const terminal = await manualCloudDeviceLogin({role, stateRoot, modules, baseUrl: "http://127.0.0.1:44123", approve: async () => {}, localClient: {send: async () => {throw new Error("human login reached kernel enrollment")}}, requests: modules.requests})
    terminals.push(terminal)
    assert.equal(terminal.profile.clientId, `cli:${terminal.identity.publicKeyThumbprint}`)
    assert.equal(terminal.profile.cloudSessionToken, undefined)
    assert.equal(terminal.cloudSessionToken, undefined)
    assert.equal((await stat(terminal.client.store.filePath)).mode & 0o777, 0o600)
    assert.ok(await terminal.client.store.load(), "CLIENT authority is persisted privately")
  }
  assert.notEqual(terminals[0].identity.publicKeyThumbprint, terminals[1].identity.publicKeyThumbprint)
  assert.notEqual(terminals[0].client.store.filePath, terminals[1].client.store.filePath)
  assert.deepEqual(calls, ["/auth/device/start", "/auth/device/poll", "/auth/device/start", "/auth/device/poll"])
})

test("MP-08 / MP-10 / MP-11: hosted commands enroll once then sign in owner without a human token in kernel responses", async t => {
  const stateRoot = await mkdtemp(path.join(tmpdir(), "chariox-hosted-composition-"))
  t.after(() => rm(stateRoot, {recursive: true, force: true}))
  const terminal = createCloudDrillClient(modules, stateRoot, "owner")
  t.after(() => terminal.client.stop())
  const calls = [], ipc = [], notices = [], profileRef = {current: null}
  mockClientLogin(t, calls)
  const profile = {api_url: "http://127.0.0.1:44123", email: "owner@example.test", account_id: "account", user_id: "owner", account_slug: "owner", realm_id: "realm", relay_url: "wss://relay.test", issuer_id: "issuer", machine_id: "machine", kernel_id: "kernel", kernel_enrolled: true}
  const localClient = {send: async request => {
    const variant = Object.keys(request)[0]; ipc.push(variant)
    if (variant === "RelayStatus") return {RelayStatus: {status: {machine_id: "machine", daemon_id: "kernel", connected: true}}}
    if (variant === "ConnectCloudRelay") return {CloudRelayConnected: {profile, status: {connected: true}}}
    if (variant === "CloudRelayStatus") return {CloudRelayStatus: {profile}}
    if (variant === "StartCloudRelayLogin") return {CloudRelayLoginStarted: {login: {api_url: profile.api_url, device_code: "synthetic-kernel-device", user_code: "PUBLIC", verification_url: "http://127.0.0.1/activate", expires_at: new Date(Date.now()+60_000).toISOString(), interval_seconds: 1}}}
    if (variant === "PollCloudRelayLogin") return {CloudRelayLoginPolled: {result: {status: "approved", profile}}}
    throw new Error(`unexpected kernel request: ${variant}`)
  }}
  const handlers = modules.createCommandActionHandlers(createHostedCommandDeps({workspace: stateRoot, localClient, requests: modules.requests, profileRef, notices, ownerAccountSlug: "owner", terminal, modules, baseUrl: profile.api_url, approve: async () => {}}))
  await handlers.handleCloudCommand({kind: "cloud", raw: "/cloud link", args: ["link"]})
  const kernelProfile = profileRef.current
  assert.equal(kernelProfile.kernelEnrolled, true)
  await handlers.handleCloudCommand({kind: "cloud", raw: "/cloud login", args: ["login"]})
  assert.equal(ipc.filter(variant => variant === "StartCloudRelayLogin").length, 1)
  assert.equal(ipc.filter(variant => variant === "PollCloudRelayLogin").length, 1)
  assert.deepEqual(calls, ["/auth/device/start", "/auth/device/poll"])
  assert.equal((await terminal.client.profile()).clientId, `cli:${terminal.identity.publicKeyThumbprint}`)
  assert.equal(profileRef.current, kernelProfile, "human login does not overwrite kernel enrollment")
  assert.equal(kernelProfile.cloudSessionToken, undefined)
})

test("MP-08 / MP-10 / MP-11: hosted multiuser scenario uses private CLIENT collaboration and key-bound session grants", async t => {
  const stateRoot = await mkdtemp(path.join(tmpdir(), "chariox-hosted-multiuser-"))
  t.after(() => rm(stateRoot, {recursive: true, force: true}))
  const terminals = {}, requests = [], grantClients = [], loggedOut = []
  for (const role of ["owner", "peer", "third"]) {
    const terminal = createCloudDrillClient(modules, stateRoot, role)
    t.after(() => terminal.client.stop())
    const profile = {apiUrl: "http://127.0.0.1:44123", accountId: `account-${role}`, userId: role, accountSlug: role, clientId: `cli:${terminal.identity.publicKeyThumbprint}`, realmId: `realm-${role}`, email: `${role}@example.test`, relayUrl: "wss://relay.test", issuerId: "issuer"}
    await terminal.client.store.saveLogin({profile, clientId: profile.clientId, publicKeyThumbprint: terminal.identity.publicKeyThumbprint, accessToken: `synthetic-${role}-access`, refreshCredential: `synthetic-${role}-refresh`, expiresAtMs: Date.now()+300_000})
    terminals[role] = {...terminal, profile}
  }
  t.mock.method(globalThis, "fetch", async (url, init) => {
    const route = new URL(url).pathname, body = JSON.parse(init.body)
    const role = ["owner", "peer", "third"].find(role => body.sessionToken === `synthetic-${role}-access`)
    assert.ok(role, "human HTTP uses a private CLIENT credential")
    assert.equal(body.kernelCredential, undefined)
    assert.equal(body.machineCredential, undefined)
    if (route === "/sessions/invites") return Response.json({inviteId: "cloud-invite", inviteToken: "synthetic-invite", sessionId: "session", accountId: "account-owner", createdByUserId: "owner"})
    if (route.endsWith("/accept")) return Response.json({sessionId: "session", accountId: "account-owner", userId: role})
    if (route === "/relay/token") {
      assert.equal(body.accountId, "account-owner")
      assert.equal(body.realmId, "realm-owner")
      assert.equal(body.sessionId, "session")
      assert.equal(body.clientId, terminals[role].profile.clientId)
      assert.equal(body.subject, body.clientId)
      assert.equal(body.publicKeyThumbprint, terminals[role].identity.publicKeyThumbprint)
      assert.deepEqual(body.allowedTargets, ["home-kernel"])
      const token = `fixture.${Buffer.from(JSON.stringify({public_key_thumbprint: body.publicKeyThumbprint, allowed_targets: body.allowedTargets, session_id: body.sessionId})).toString("base64url")}.fixture`
      return Response.json({token, expiresAt: new Date(Date.now()+300_000).toISOString()})
    }
    assert.equal(route, "/auth/logout")
    assert.equal(body.revokeClient, true)
    loggedOut.push(role)
    return Response.json({})
  })
  const scenarioModules = {...modules, LocalIpcClient: class {
    constructor(url, options) { assert.equal(url, "wss://relay.test"); grantClients.push(options) }
    startRelayAuthRenewal() {}
    async send(request) { requests.push(Object.keys(request)[0]); throw new Error("synthetic kernel membership denial") }
    async close() {}
  }}
  await assert.rejects(runHostedMultiUserAssertions({
    requests: modules.requests, ownerTerminal: terminals.owner, ownerProfile: terminals.owner.profile,
    ownerRemoteClient: {send: async request => {requests.push(Object.keys(request)[0]); assert.ok(request.CreateSessionInvite, "only runtime invite creation uses IPC"); return {SessionInviteCreated: {invite: {invite_token: "local-invite"}}}}},
    modules: scenarioModules, stateRoot, homeDaemonId: "home-kernel", workspace: stateRoot, session: {id: "session"},
    log: () => {}, assert, unwrap, installSendRetry, connectSessionScopedCloudClient, cleanupHostedCloudTerminal,
    manualCloudDeviceLogin: async input => {assert.equal(input.localClient, undefined); assert.equal(input.requests, undefined); return terminals[input.role]},
  }), /synthetic kernel membership denial/)
  assert.deepEqual(requests, ["CreateSessionInvite", "JoinSessionInvite"])
  assert.equal(grantClients.length, 3)
  for (const [index, role] of ["owner", "peer", "third"].entries()) {
    assert.equal(grantClients[index].relayIdentity, terminals[role].identity)
    assert.equal(grantClients[index].targetDaemonId, "home-kernel")
  }
  assert.deepEqual(loggedOut.sort(), ["peer", "third"])
  assert.equal(await terminals.peer.client.store.load(), null)
  assert.equal(await terminals.third.client.store.load(), null)
})

test("MP-08 / MP-10 / MP-11: hosted --check loads the migrated composition without live services", async () => {
  const result = await promisify(execFile)(process.execPath, [fileURLToPath(new URL("../live-hosted-cloud-relay-drill.mjs", import.meta.url)), "--check"])
  assert.match(result.stdout, /module-composition-ok/)
})

test("MP-11: hosted helper rejects invalid signal targets and omits private response details", async () => {
  for (const pid of [undefined, NaN, -1, 0, 1]) {
    await assert.rejects(terminateChild({pid, exitCode: null, signalCode: null, kill: () => {throw new Error("unsafe target was signalled")}}), /refusing to signal/)
  }
  assert.throws(() => drillAssert(false, "synthetic failure", {credential: "synthetic-private-value"}), error => error.message === "synthetic failure")
})

test("MP-08 / MP-10 / MP-11: hosted remote CLI pairs with its own CLIENT profile rather than forwarding the owner's grant", async () => {
  let command, pairingRequests = 0, prepared = false, cleaned = false
  await assert.rejects(runHostedRemoteCliAssertions({
    requests: modules.requests,
    homeClient: {send: async request => {assert.ok(request.CreateTerminalPairingLink); pairingRequests++; return {TerminalPairingLinkCreated: {pairing: {pairing_link: "synthetic-pairing-link"}}}}},
    repoRoot: "/synthetic-repo", remoteCliRepo: "/synthetic-remote-repo", remoteCliHost: "synthetic-host",
    log: () => {}, assert, unwrap, shellQuote: value => `'${value}'`, sshArgs: value => {command = value; return []},
    spawnProcess: () => {throw new Error("synthetic launch boundary")}, terminateChild: async () => {}, runSsh: async () => {}, allowDevStubProvider: async () => {},
    prepareRemoteTerminal: async () => {
      prepared = true
      return { environment: "export CHARIOX_HOME='/isolated-receiver'", cleanup: async () => { cleaned = true } }
    },
  }), /synthetic launch boundary/)
  assert.equal(prepared, true, "receiver CLIENT login must finish before TUI launch")
  assert.equal(cleaned, true, "receiver logout must run even when TUI launch fails")
  assert.equal(pairingRequests, 1)
  assert.match(command, /--terminal-pairing-link 'synthetic-pairing-link'/)
  assert.doesNotMatch(command, /--relay-token/)
  assert.match(command, /CHARIOX_HOME='\/isolated-receiver'/)
})

test("MP-08 / MP-10 / MP-11: receiver device login feeds actual CLI pairing bootstrap and acknowledged logout", async t => {
  const root = await mkdtemp(path.join(tmpdir(), "kauthval-receiver-bootstrap-"))
  const previousHome = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = path.join(root, "home/.chariox")
  t.after(async () => {
    if (previousHome === undefined) delete process.env.CHARIOX_HOME
    else process.env.CHARIOX_HOME = previousHome
    await rm(root, {recursive: true, force: true})
  })
  const calls = []
  let identity, grantKey, logouts = 0
  t.mock.method(globalThis, "fetch", async (url, init) => {
    const route = new URL(url).pathname, body = JSON.parse(init.body)
    calls.push(route)
    if (route === "/auth/device/start") {
      assert.equal(body.enrollmentKind, "CLIENT")
      identity = body
      return Response.json({deviceCode: "synthetic-device", userCode: "PUBLIC", verificationUrl: "http://127.0.0.1/activate", expiresAt: new Date(Date.now()+60_000).toISOString(), intervalSeconds: 1})
    }
    if (route === "/auth/device/poll") return Response.json({status: "approved", profile: {enrollmentKind: "CLIENT", publicKeyThumbprint: identity.publicKeyThumbprint, clientId: identity.clientId, email: "owner@example.test", accountSlug: "owner", accountId: "account", userId: "owner", realmId: "realm", relayUrl: "wss://relay.test", issuerId: "issuer"}, cloudSessionToken: "synthetic-human-access", refreshCredential: "synthetic-refresh", cloudSessionExpiresAt: new Date(Date.now()+300_000).toISOString()})
    if (route === "/relay/token") {
      assert.equal(body.clientId, identity.clientId)
      assert.deepEqual(body.allowedTargets, ["home-kernel"])
      grantKey = body.publicKeyThumbprint
      return Response.json({token: `fixture.${Buffer.from(JSON.stringify({public_key_thumbprint: grantKey})).toString("base64url")}.fixture`, expiresAt: new Date(Date.now()+300_000).toISOString()})
    }
    assert.equal(route, "/auth/logout")
    assert.equal(body.clientId, identity.clientId)
    assert.equal(body.revokeClient, true)
    logouts++
    return Response.json(null)
  })
  const pairingLink = "chariox://terminal-pairing/synthetic"
  const options = {relayUrl: "wss://relay.test", relayToken: "cloud-client-token-required", targetDaemonId: "home-kernel", clientId: "receiver", workspace: root, worktree: root}
  let joins = 0, clients = 0
  const fakeClient = {close: async () => {}, send: async request => {
    assert.ok(request.JoinTerminalPairingLink)
    assert.equal(request.JoinTerminalPairingLink.public_key_thumbprint, grantKey)
    joins++
    return {TerminalPairingLinkJoined: {kernel_pairing: true, terminal: {terminal_id: "receiver"}, pairing: {target_daemon_id: "home-kernel", relay_url: options.relayUrl, subject_id: "receiver", public_key_thumbprint: grantKey}}}
  }}
  const deps = {...defaultCliRuntimeBootstrapDeps, parseArgs: () => ({...options}),
    loadPreferences: async () => ({}), applyProviderPreferenceDefaults: () => {},
    createClient: () => { clients++; return fakeClient },
    inferWorkspaceTargetsFromLaunchDirectory: async () => ({workspace: root, worktree: root}),
    clearWaitingRoomWorktreeInventory: () => {}, loadThemeRegistry: async () => ({}),
    isNoArgDefaultKernelLaunch: () => false, maybeResize: async () => {},
    bootstrapAttachedSession: async () => ({client: fakeClient, sessionId: "session", transcript: []}),
  }
  const boot = () => bootstrapCliRuntime({argv: ["--terminal-pairing-link", pairingLink], cwd: root}, deps)
  await assert.rejects(boot(), /signed-in terminal/)
  assert.equal(clients, 0, "fresh receiver rejects before IPC")
  await runRemoteTerminalLogin("login", root, "http://127.0.0.1:44123", "account")
  assert.equal((await boot()).kind, "ready")
  assert.equal(joins, 1)
  assert.equal(grantKey, identity.publicKeyThumbprint)
  assert.equal((await stat(path.join(process.env.CHARIOX_HOME, "relay/cloud-client.json"))).mode & 0o777, 0o600)
  await runRemoteTerminalLogin("logout", root)
  assert.equal(logouts, 1)
  assert.deepEqual(calls, ["/auth/device/start", "/auth/device/poll", "/relay/token", "/auth/logout"])
})


test("MP-08 / MP-10 / MP-11: real-provider pairing prepares its own receiving CLIENT and cleans it on launch failure", async () => {
  let prepared = false, cleaned = false, command, server;
  try {
    await assert.rejects(runHostedRemoteCliPairingAssertions({
      requests: modules.requests,
      homeClient: {send: async () => ({TerminalPairingLinkCreated: {pairing: {pairing_link: "synthetic-pairing", terminal_id: "terminal"}}})},
      workspace: "/workspace", kernelUrl: "ws://local", cliRoot: "/cli", repoRoot: "/repo",
      remoteCliRepo: "/receiver", remoteCliHost: "receiver", remoteCliPairingProvider: "opencode",
      remoteCliPairingModel: "opencode-go/gpt-5.6-luna", remoteCliPairingEffort: "default",
      apiUrl: "https://cloud.test", ownerAccountSlug: "owner", ownerAccountId: "account",
      log: () => {}, assert, unwrap, shellQuote: value => `'${value}'`,
      sshArgs: value => {command = value; return []},
      runSsh: async () => ({code: 0}), terminateChild: async () => {},
      spawnProcess: (program, args) => {
        if (program === "ssh") throw new Error("synthetic remote launch boundary");
        const socket = process.platform === "linux"
          ? args[2].match(/'--automation-socket' '([^']+)'/)[1]
          : args[args.indexOf("--automation-socket") + 1];
        server = net.createServer(connection => connection.on("data", chunk => {
          const request = JSON.parse(chunk.toString());
          connection.write(JSON.stringify({id: request.id, ok: true,
            data: {session: {id: "session", focusedAgentId: "agent"}}}) + "\n");
        })).listen(socket);
        return {};
      },
      prepareRemoteTerminal: async options => {
        prepared = true;
        assert.equal(options.apiUrl, "https://cloud.test");
        assert.equal(options.ownerAccountId, "account");
        assert.match(options.remoteRoot, /^\/var\/tmp\/chariox-kauthval-hosted-remote-cli-\d+-\d+$/);
        return {environment: "export CHARIOX_HOME='/isolated-pairing-receiver'",
          cleanup: async () => {cleaned = true}};
      },
    }), /synthetic remote launch boundary/);
    assert.equal(prepared, true);
    assert.equal(cleaned, true);
    assert.match(command, /CHARIOX_HOME='\/isolated-pairing-receiver'/);
    assert.match(command, /--terminal-pairing-link 'synthetic-pairing'/);
    assert.doesNotMatch(command, /--relay-token/);
  } finally { await new Promise(resolve => server ? server.close(resolve) : resolve()); }
});
