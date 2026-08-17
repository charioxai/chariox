import test from "node:test"
import assert from "node:assert/strict"

import {
  applyProviderPreferenceDefaults,
  parseArgs,
  resolveConfiguredCloudRelayApiUrl,
} from "./cli-options.js"

test("parseArgs applies terminal pairing links", () => {
  const payload = Buffer.from(JSON.stringify({
    relay_url: "wss://relay.example.test",
    relay_token: "relay-token",
    target_daemon_id: "kernel-1",
    target_daemon_alias: "devbox",
    terminal_id: "terminal-1",
  })).toString("base64url")

  assert.deepEqual(parseArgs([`chariox-terminal-pair-v1.${payload}`]), {
    clientId: "terminal-1",
    model: "default",
    accountProfile: "default",
    effort: "",
    relayUrl: "wss://relay.example.test",
    relayToken: "relay-token",
    targetDaemonId: "kernel-1",
    targetDaemonAlias: "devbox",
  })
})

test("parseArgs rejects invalid option combinations", () => {
  assert.throws(
    () => parseArgs(["--relay-url", "wss://relay.example.test"]),
    /--relay-url requires --relay-token/,
  )
  assert.throws(
    () => parseArgs(["--create-session", "--session", "session-1"]),
    /--create-session cannot be used together with --session/,
  )
})

test("parseArgs help lists remote runtime once next to kernel health", () => {
  const previousWrite = process.stdout.write
  const previousExit = process.exit
  let output = ""
  ;(process.stdout.write as unknown as (chunk: string) => boolean) = (chunk: string) => {
    output += chunk
    return true
  }
  ;(process.exit as unknown as (code?: number) => never) = ((code?: number) => {
    throw Object.assign(new Error("process.exit"), { code })
  }) as (code?: number) => never

  try {
    assert.throws(() => parseArgs(["--help"]), /process\.exit/)
  } finally {
    process.stdout.write = previousWrite
    process.exit = previousExit
  }

  assert.match(output, /\/kernel health\s+show runtime health, remote readiness, and invariants/)
  assert.match(output, /\/kernel remote-runtime\s+show home authority, worker runs, slices, home-proxy tools, and live sync readiness/)
  assert.match(output, /\/session status\s+show owner, placement, live sync, extensions, and blockers/)
  assert.match(output, /\/workspace sync \.\.\.\s+manage live sync status, session mode, global default, links, and diagnostics/)
  assert.match(output, /\/provider processes \[n\]\s+list daemon-tracked provider processes/)
  assert.match(output, /\/provider processes teardown <n>\s+tear down safe daemon-tracked provider processes/)
  assert.match(output, /\/extension import\s+import provider MCPs and skills into Chariox/)
  assert.match(output, /\/extension grant\s+grant mcp, skill, script, or connector capabilities to an agent/)
  assert.match(output, /\/extension revoke\s+revoke an extension from an agent/)
  assert.match(output, /\/extension grants\s+show worker-local, home-proxy, and skill snapshot grants/)
  assert.match(output, /\/extension sync-status\s+show home-proxy manifest sync and recovery for an agent/)
  assert.match(output, /\/extension sync-retry\s+retry home-proxy manifest projection after worker reconnect/)
  assert.match(output, /\/extension audit\s+show home extension audit events and denials for an agent/)
  assert.match(output, /\/agent spawn \[a\] \[m\] \[--dir d\] \[--worktree d --branch b\] \[--machine r\|--kernel k\|--slice off\|new\|s\] \[--slice-display headless\|headed\] spawn a local, remote, or slice agent/)
  assert.match(output, /\/slice state \[s\]\s+show saved slice state and restart requirements/)
  assert.match(output, /\/slice save-state \[s\]\s+save slice state after shutdown or agent restart/)
  assert.match(output, /\/slice backup \[s\]\s+create a recoverable slice state backup/)
  assert.match(output, /\/slice reset-state \[s\]\s+reset saved slice state after agents are detached/)
  assert.match(output, /\/slice auth import\s+copy this machine's provider credentials into the slice/)
  assert.match(output, /\/slice auth remove\s+remove slice-local provider credentials and account summary/)
  assert.match(output, /\/slice auth login\s+start provider login inside the slice for a different account/)
  assert.match(output, /\/slice auth alias\s+set a Chariox display alias for the slice account/)
  assert.equal(output.match(/\/kernel health/g)?.length, 1)
  assert.equal(output.match(/\/kernel remote-runtime/g)?.length, 1)
})

test("resolveConfiguredCloudRelayApiUrl prefers env and trims trailing slashes", () => {
  const previous = process.env.CHARIOX_CLOUD_API_URL
  process.env.CHARIOX_CLOUD_API_URL = "https://cloud.example.test///"
  try {
    assert.equal(
      resolveConfiguredCloudRelayApiUrl({
        relay: {
          cloud: {
            apiUrl: "https://profile.example.test",
            email: "user@example.test",
            accountId: "account-1",
            userId: "user-1",
            accountSlug: "account",
            realmId: "realm-1",
            relayUrl: "wss://relay.example.test",
            issuerId: "issuer-1",
          },
        },
      }),
      "https://cloud.example.test",
    )
  } finally {
    if (previous === undefined) {
      delete process.env.CHARIOX_CLOUD_API_URL
    } else {
      process.env.CHARIOX_CLOUD_API_URL = previous
    }
  }
})

test("applyProviderPreferenceDefaults fills default model and blank effort from provider preferences", () => {
  const options = parseArgs(["--provider", "codex"])

  assert.equal(
    applyProviderPreferenceDefaults(options, {
      providers: {
        codex: {
          model: "gpt-5.1",
          effort: "high",
        },
      },
    }).model,
    "gpt-5.1",
  )
  assert.equal(options.effort, "high")
})

test("applyProviderPreferenceDefaults preserves explicit model and effort", () => {
  const options = parseArgs(["--provider", "codex", "--model", "gpt-5.2", "--effort", "medium"])

  applyProviderPreferenceDefaults(options, {
    providers: {
      codex: {
        model: "gpt-5.1",
        effort: "high",
      },
    },
  })

  assert.equal(options.model, "gpt-5.2")
  assert.equal(options.effort, "medium")
})
