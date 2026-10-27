import assert from "node:assert/strict"
import test from "node:test"

import {
  parseArgs,
  remoteAcceptanceCommand,
  runAcceptance,
  validateOptions,
  validatePreflightEvidence,
} from "../deploy/event-publication/hetzner-acceptance.mjs"

const options = {
  preflight: "/tmp/preflight.json",
  runId: "event-acceptance-001",
  component: "github",
  expectedAegs: [],
  aedsHost: "root@aeds.example",
  aegsHost: "root@aegs.example",
  relayHost: "root@relay.example",
  sshKey: "/tmp/key",
  aedsSshKey: "",
  aegsSshKey: "",
  aedsUrl: "https://aeds.example",
  aegsUrl: "https://github-events.example",
  evidenceDir: "/tmp/evidence",
  executeRestarts: false,
}

function preflight() {
  return {
    schemaVersion: 1,
    kind: "chariox-event-publication-hetzner-preflight",
    runId: options.runId,
    source: { revision: "a".repeat(40), dirty: false },
    separation: { aedsAndAegs: true, eventServicesAndRelay: true },
    hosts: {
      aeds: { sshTarget: options.aedsHost, machineId: "machine-a" },
      aegs: { sshTarget: options.aegsHost, machineId: "machine-b" },
      relay: { sshTarget: options.relayHost, machineId: "machine-c" },
    },
  }
}

test("arguments require exact hosts, TLS origins, and a first-wave component", () => {
  const parsed = parseArgs([
    "--preflight", options.preflight,
    "--run-id", options.runId,
    "--component", options.component,
    "--aeds-host", options.aedsHost,
    "--aegs-host", options.aegsHost,
    "--relay-host", options.relayHost,
    "--ssh-key", options.sshKey,
    "--aeds-url", options.aedsUrl,
    "--aegs-url", options.aegsUrl,
    "--execute-restarts",
  ])
  assert.equal(parsed.executeRestarts, true)
  assert.doesNotThrow(() => validateOptions(parsed))
  assert.throws(() => validateOptions({ ...options, component: "dummy" }), /component/)
  assert.throws(() => validateOptions({ ...options, aedsUrl: "http://aeds.example" }), /HTTPS/)
  assert.throws(
    () => validateOptions({ ...options, aedsUrl: "https://aeds.example/not-an-origin" }),
    /HTTPS origin/,
  )
  assert.throws(
    () => validateOptions({ ...options, aegsHost: options.relayHost }),
    /relay host/,
  )
  assert.doesNotThrow(() => validateOptions({
    ...options,
    expectedAegs: ["github", "jira", "slack"],
  }))
  assert.throws(
    () => validateOptions({ ...options, expectedAegs: ["jira"] }),
    /must include --component/,
  )
  assert.doesNotThrow(() => validateOptions({
    ...options,
    sshKey: "",
    aedsSshKey: "/tmp/aeds-key",
    aegsSshKey: "/tmp/aegs-key",
  }))
  assert.throws(
    () => validateOptions({ ...options, sshKey: "", aedsSshKey: "/tmp/aeds-key" }),
    /both --aeds-ssh-key and --aegs-ssh-key/,
  )
})

test("acceptance is bound to clean separated preflight evidence", () => {
  assert.doesNotThrow(() => validatePreflightEvidence(preflight(), options))
  assert.throws(
    () => validatePreflightEvidence(
      { ...preflight(), hosts: { ...preflight().hosts, aegs: { ...preflight().hosts.aegs, machineId: "machine-a" } } },
      options,
    ),
    /one machine/,
  )
  assert.throws(
    () => validatePreflightEvidence({ ...preflight(), source: { dirty: true } }, options),
    /clean/,
  )
})

test("remote commands fence machine, role, and the exact concurrent AEGS set", () => {
  const readOnly = remoteAcceptanceCommand({
    role: "aegs",
    component: "github",
    activeComponents: ["github", "jira", "slack"],
    machineId: "machine-b",
    url: options.aegsUrl,
    restart: false,
  })
  assert.match(readOnly, /cat \/etc\/machine-id/)
  assert.match(readOnly, /host-role/)
  assert.match(readOnly, /chariox-aegs-github\.service/)
  assert.match(readOnly, /active_units/)
  assert.match(readOnly, /chariox-aegs-jira\.service/)
  assert.match(readOnly, /chariox-aegs-slack\.service/)
  assert.match(readOnly, /sort/)
  assert.doesNotMatch(readOnly, /systemctl restart/)
  assert.doesNotMatch(readOnly, /\brm\b|docker system prune|docker compose down/)

  const restart = remoteAcceptanceCommand({
    role: "aeds",
    component: "github",
    machineId: "machine-a",
    url: options.aedsUrl,
    restart: true,
  })
  assert.match(restart, /systemctl restart/)
  assert.match(restart, /chariox-aeds\.service/)
  assert.doesNotMatch(restart, /\breboot\b|\brm\b|docker system prune/)
})

test("acceptance checks AEDS then the expected AEGS set and retains bounded evidence", async () => {
  const calls = []
  let written = ""
  const result = await runAcceptance(options, {
    readFile: async () => JSON.stringify(preflight()),
    revision: "a".repeat(40),
    dirty: false,
    runSsh: async (host, key, command) => {
      calls.push({ host, key, command })
      return `host=${host}`
    },
    mkdir: async () => {},
    writeFile: async (_path, value) => { written = value },
  })
  assert.deepEqual(calls.map((call) => call.host), [options.aedsHost, options.aegsHost])
  assert.equal(result.record.restartMode, "read-only")
  assert.equal(result.record.component, "github")
  assert.deepEqual(result.record.expectedAegs, ["github"])
  assert.match(written, /chariox-event-publication-hetzner-acceptance/)
  assert.doesNotMatch(written, /sshKey/)
})

test("acceptance uses role-specific SSH keys without retaining their paths", async () => {
  const roleSpecific = {
    ...options,
    sshKey: "",
    aedsSshKey: "/tmp/aeds-key",
    aegsSshKey: "/tmp/aegs-key",
  }
  const calls = []
  let written = ""
  await runAcceptance(roleSpecific, {
    readFile: async () => JSON.stringify(preflight()),
    revision: "a".repeat(40),
    dirty: false,
    runSsh: async (host, key) => {
      calls.push({ host, key })
      return `host=${host}`
    },
    mkdir: async () => {},
    writeFile: async (_path, value) => { written = value },
  })
  assert.deepEqual(calls.map((call) => call.key), ["/tmp/aeds-key", "/tmp/aegs-key"])
  assert.doesNotMatch(written, /aeds-key|aegs-key/)
})

test("acceptance refuses stale preflight revisions and dirty execution state", async () => {
  const dependencies = {
    readFile: async () => JSON.stringify(preflight()),
    revision: "b".repeat(40),
    dirty: false,
    runSsh: async () => assert.fail("SSH must not run"),
  }
  await assert.rejects(() => runAcceptance(options, dependencies), /revision/)
  await assert.rejects(
    () => runAcceptance(options, { ...dependencies, revision: "a".repeat(40), dirty: true }),
    /clean OSS worktree/,
  )
})
