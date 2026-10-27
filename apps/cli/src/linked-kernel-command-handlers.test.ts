import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcError } from "@chariox/kernel-client/ipc"
import type { GatewayKernelRecord } from "@chariox/kernel-client/kernel-types"
import type { LocalIpcClient } from "./ipc.js"
import { forgetGatewayKernel, joinKernelLink, listGatewayKernels } from "./kernel-gateway-api.js"
import { handleKernelSlashCommand } from "./kernel-command-handlers.js"
import { handleRemoteMachineSlashCommand } from "./remote-machine-command-handlers.js"

const LINK = "chariox-terminal-pair-v1.eyJzZWNyZXQiOiJsaW5rIn0"
const TOKEN = "client-token-do-not-print"
const TOKEN_VARIABLE = "CHARIOX_TEST_OPERATOR_CLIENT_TOKEN"

const builder: GatewayKernelRecord = {
  name: "builder", kernel_id: "kernel-b", machine_id: "machine-b", machine_alias: null, label: "builder",
  status: "online", last_seen_ms: 1, gateway_ready: true, via: "self_hosted",
}

test("kernel add pairs with a link and reports the kernel's status", async () => {
  const harness = createHarness()
  await harness.run("add", LINK)
  assert.deepEqual(harness.joins, [{ link: LINK, relayToken: null }])
  assert.deepEqual(harness.footers, [{ message: "Paired with builder (online)", tone: "info" }])
  harness.assertNothingLeaked()
})

test("kernel add sends the token held by the named environment variable and never prints it", async () => {
  process.env[TOKEN_VARIABLE] = TOKEN
  try {
    const harness = createHarness()
    await harness.run("add", LINK, "--relay-token-env", TOKEN_VARIABLE)
    assert.deepEqual(harness.joins, [{ link: LINK, relayToken: TOKEN }])
    assert.deepEqual(harness.footers, [{ message: "Paired with builder (online)", tone: "info" }])
    harness.assertNothingLeaked()
  } finally { delete process.env[TOKEN_VARIABLE] }
})

test("kernel add says which environment variable is unset and does not call the kernel", async () => {
  delete process.env[TOKEN_VARIABLE]
  const harness = createHarness()
  await harness.run("add", LINK, "--relay-token-env", TOKEN_VARIABLE)
  assert.deepEqual(harness.joins, [])
  assert.equal(harness.footers[0]?.tone, "error")
  assert.match(harness.footers[0]!.message, new RegExp(`environment variable ${TOKEN_VARIABLE} is not set`))
  harness.assertNothingLeaked()
})

test("kernel add refuses a pasted token in place of a variable name without echoing it", async () => {
  const harness = createHarness()
  await harness.run("add", LINK, "--relay-token-env", TOKEN + "!")
  assert.deepEqual(harness.joins, [])
  assert.equal(harness.footers[0]?.tone, "error")
  harness.assertNothingLeaked()
})

test("kernel add usage never echoes the link", async () => {
  const harness = createHarness()
  await harness.run("add", LINK, "--relay-token-env")
  await harness.run("add", LINK, "--other", "VAR")
  await harness.run("add")
  assert.deepEqual(harness.joins, [])
  assert.deepEqual(harness.footers.map(footer => footer.message), Array(3).fill("usage: /kernel add <link> [--relay-token-env VAR]"))
  harness.assertNothingLeaked()
})

test("kernel add shows the kernel's refusal verbatim without the transport wrapper", async () => {
  const refusal = "kernel_join_refused: Could not pair with that kernel: make a fresh link on that kernel with chariox pair and add it again"
  const harness = createHarness({
    join: async () => { throw new LocalIpcError("handle kernel response", refusal, "kernel_join_refused") },
  })
  await harness.run("add", LINK)
  assert.deepEqual(harness.footers, [{ message: refusal, tone: "error" }])
  harness.assertNothingLeaked()
})

test("machine add still reaches the SSH installer and no longer pairs a link", async () => {
  const harness = createHarness()
  const ssh: unknown[] = []
  const deps = {
    ...harness.deps,
    addSshMachine: async (host: string) => { ssh.push(host); return { install_id: "i", status: "ready", kernel_id: null, machine_id: null, release_digest: "sha256:x", state_retained: false } },
  }
  await handleRemoteMachineSlashCommand(deps, { kind: "machine", raw: "/machine add ssh lan", args: ["add", "ssh", "lan"] })
  assert.deepEqual(ssh, ["lan"])
  await handleRemoteMachineSlashCommand(deps, { kind: "machine", raw: `/machine add ${LINK}`, args: ["add", LINK] })
  assert.deepEqual(harness.joins, [])
})

test("kernel linked lists name, status and transport", async () => {
  const harness = createHarness({ kernels: [builder, { ...builder, name: "cloud-box", status: "offline", via: "hosted" }] })
  await harness.run("linked")
  assert.deepEqual(harness.footers, [{ message: "builder online via=self_hosted\ncloud-box offline via=hosted", tone: "info" }])
})

test("kernel linked says how to pair when nothing is linked", async () => {
  const harness = createHarness({ kernels: [] })
  await harness.run("linked")
  assert.deepEqual(harness.footers, [{ message: "no linked kernels; pair one with /kernel add <link>", tone: "info" }])
})

test("kernel unlink forgets a paired kernel by name and shows a refusal", async () => {
  const harness = createHarness()
  await harness.run("unlink", "builder")
  assert.deepEqual(harness.forgotten, ["builder"])
  assert.deepEqual(harness.footers, [{ message: "Forgot linked kernel builder", tone: "info" }])

  const refused = createHarness({ forget: async () => { throw new LocalIpcError("handle kernel response", "kernel_not_found: No linked kernel is called ghost; run /kernel linked", "kernel_not_found") } })
  await refused.run("unlink", "ghost")
  assert.deepEqual(refused.footers, [{ message: "kernel_not_found: No linked kernel is called ghost; run /kernel linked", tone: "error" }])
  await refused.run("unlink")
  assert.equal(refused.footers.at(-1)?.message, "usage: /kernel unlink <name>")
})

test("the kernel requests carry the link and token and read the answers", async () => {
  const sent: unknown[] = []
  const answers = [
    { GatewayKernelJoined: { kernel: builder } },
    { GatewayKernelsListed: { kernels: [builder] } },
    { GatewayKernelForgotten: { kernel_id: "kernel-b" } },
  ]
  const client = { send: async (request: unknown) => { sent.push(request); return answers.shift() } } as unknown as LocalIpcClient
  assert.deepEqual(await joinKernelLink(client, LINK, TOKEN), builder)
  assert.deepEqual(await listGatewayKernels(client), [builder])
  await forgetGatewayKernel(client, "builder")
  assert.deepEqual(sent, [
    { JoinKernelLink: { link: LINK, relay_token: TOKEN } },
    { ListGatewayKernels: {} },
    { ForgetGatewayKernel: { name: "builder" } },
  ])
})

function createHarness(options: {
  kernels?: GatewayKernelRecord[]
  join?: (link: string, relayToken: string | null) => Promise<GatewayKernelRecord>
  forget?: (name: string) => Promise<void>
} = {}) {
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const joins: Array<{ link: string; relayToken: string | null }> = []
  const forgotten: string[] = []
  const deps = {
    flashFooter: (message: string, tone: "info" | "error") => { footers.push({ message, tone }) },
    joinKernelLink: async (link: string, relayToken: string | null) => {
      joins.push({ link, relayToken })
      return options.join ? options.join(link, relayToken) : builder
    },
    listGatewayKernels: async () => options.kernels ?? [builder],
    forgetGatewayKernel: async (name: string) => {
      forgotten.push(name)
      await options.forget?.(name)
    },
  }
  return {
    footers, joins, forgotten, deps,
    run: (...args: string[]) => handleKernelSlashCommand(
      { ...deps, isAttached: () => false, sessionState: () => { throw new Error("no session") }, appendNotice: () => {}, transitionToNoSession: () => {} },
      { kind: "kernel", raw: `/kernel ${args.join(" ")}`, args },
    ),
    assertNothingLeaked: () => {
      const shown = JSON.stringify(footers)
      assert.ok(!shown.includes(LINK) && !shown.includes("eyJzZWNyZXQ"), "the pairing link was shown")
      assert.ok(!shown.includes(TOKEN), "the relay token was shown")
    },
  }
}
