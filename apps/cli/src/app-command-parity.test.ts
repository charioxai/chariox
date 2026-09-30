import assert from "node:assert/strict"
import test from "node:test"
import { appCommandCatalog, cliAppUsage, tuiAppHelp } from "./app-command-catalog.js"
import { parseSlashCommand, sharedShellCommandForSlashCommand } from "./commands.js"
import { runAppCommand } from "./app-command.js"
import { handleAppSlashCommand } from "./app-command-handler.js"

// One invocation of every command both surfaces share.
const shared: Record<string, string[][]> = {
  list: [["list", "--limit", "10"]],
  set: [["set"]],
  status: [["status", "install-1"]],
  journal: [["journal", "install-1"]],
  logs: [["logs", "install-1", "--after", "4"]],
  worker: [["worker", "install-1"]],
  start: [["start", "install-1"]],
  stop: [["stop", "install-1"]],
  restart: [["restart", "install-1"]],
  open: [["open", "install-1", "--session", "session-1"]],
  uninstall: [["uninstall", "install-1", "--generation", "3"]],
  automation: [
    ["automation", "list", "install-1"],
    ["automation", "add", "install-1", "reminders", "todo_due", "session-1", "workflow-1", "--scheduled"],
    ["automation", "disable", "install-1", "reminders", "2"],
  ],
  inbox: [
    ["inbox", "list", "install-1"],
    ["inbox", "add", "install-1", "route-1", "message_received", "message"],
    ["inbox", "remove", "install-1", "route-1"],
    ["inbox", "test", "install-1", "route-1", "occurrence-1", "{}"],
  ],
  connection: [
    ["connection", "list", "install-1"],
    ["connection", "grant", "install-1", "slack/team-1"],
    ["connection", "revoke", "install-1", "team-1"],
  ],
  file: [["file", "revoke", "install-1"], ["file", "revoke", "install-1", "operation-1"]],
}

async function cliRequests(args: string[]): Promise<unknown[]> {
  const requests: unknown[] = []
  await runAppCommand(["app", ...args], {
    createClient: () => ({
      send: async (request) => { requests.push(request); return {} },
      close: async () => {},
    }),
    write: () => {},
  }).catch(() => {})
  return requests
}

async function tuiRequests(args: string[]): Promise<unknown[]> {
  const requests: unknown[] = []
  await handleAppSlashCommand({
    sendAppRequest: async (request) => { requests.push(request); return {} },
    appendNotice: () => {},
    flashFooter: () => {},
  }, { kind: "app", raw: `/app ${args.join(" ")}`, args }).catch(() => {})
  return requests
}

test("every command both surfaces share sends the same kernel request from the CLI and the TUI", async () => {
  const sharedVerbs = appCommandCatalog.filter((entry) => entry.cli && entry.tui && !["install", "update"].includes(entry.verb)).map((entry) => entry.verb)
  assert.deepEqual(Object.keys(shared).sort(), [...sharedVerbs].sort())
  for (const invocations of Object.values(shared)) {
    for (const args of invocations) {
      const cli = await cliRequests(args)
      assert.equal(cli.length, 1, `chariox app ${args.join(" ")}`)
      assert.deepEqual(await tuiRequests(args), cli, `/app ${args.join(" ")}`)
    }
  }
})

test("a command of one surface names the other surface's command", async () => {
  await assert.rejects(runAppCommand(["app", "dev", "./notes"], {
    createClient: () => assert.fail("no kernel connection"),
    write: () => {},
  }), /app dev runs in a Chariox terminal: use \/app dev there/)
  for (const verb of ["create", "keygen", "manifest", "validate", "pack", "inspect"]) {
    const raw = `/app ${verb} --output notes.cxapp`
    assert.equal(sharedShellCommandForSlashCommand(raw), null, "must reach the local handler")
    const command = parseSlashCommand(raw)!
    assert.equal(command.kind, "app")
    if (command.kind !== "app") assert.fail("expected App command")
    for (const connected of [true, false]) {
      const flashes: string[] = []
      await handleAppSlashCommand({
        sendAppRequest: connected ? async () => assert.fail("no kernel request") : undefined,
        appendNotice: () => {},
        flashFooter: (message) => { flashes.push(message) },
      }, command)
      assert.deepEqual(flashes, [`app ${verb} runs in a shell: use chariox app ${verb}`])
    }
  }
})

// The command-parity snapshot: a change to either surface's commands shows up here.
test("App command parity snapshot", () => {
  const rows = appCommandCatalog.map((entry) => `${entry.verb}: cli=${entry.cli ?? "-"} | tui=${entry.tui ?? "-"}`)
  assert.deepEqual(rows, [
    "create: cli=create [options] | tui=-",
    "keygen: cli=keygen [options] | tui=-",
    "manifest: cli=manifest [options] | tui=-",
    "validate: cli=validate [options] | tui=-",
    "pack: cli=pack [options] | tui=-",
    "inspect: cli=inspect [options] | tui=-",
    'dev: cli=- | tui=dev "DIRECTORY" [--key PRIVATE] | dev stop',
    'publisher: cli=- | tui=publisher enroll "publisher.json" [--revision N] | publisher status|cancel [REVIEW-ID]',
    'install: cli=install FILE.cxapp --session SESSION | tui=install "FILE.cxapp"',
    'update: cli=update INSTALLATION FILE.cxapp --session SESSION | tui=update INSTALLATION "FILE.cxapp"',
    "operation: cli=- | tui=operation [REQUEST-ID]",
    "cancel: cli=- | tui=cancel [REQUEST-ID]",
    "list: cli=list [--after ID] [--limit 1..100] | tui=list [--after ID] [--limit 1..100]",
    "set: cli=set | tui=set",
    "status: cli=status INSTALLATION | tui=status INSTALLATION",
    "journal: cli=journal INSTALLATION | tui=journal INSTALLATION",
    "logs: cli=logs INSTALLATION [--after SEQUENCE] | tui=logs INSTALLATION [--after SEQUENCE]",
    "worker: cli=worker INSTALLATION | tui=worker INSTALLATION",
    "start: cli=start INSTALLATION | tui=start INSTALLATION",
    "stop: cli=stop INSTALLATION | tui=stop INSTALLATION",
    "restart: cli=restart INSTALLATION | tui=restart INSTALLATION",
    "open: cli=open INSTALLATION --session SESSION | tui=open INSTALLATION [--session SESSION]",
    "uninstall: cli=uninstall INSTALLATION [--generation N] [--delete-data] | tui=uninstall INSTALLATION [--generation N] [--delete-data]",
    "automation: cli=automation list|add|disable INSTALLATION … | tui=automation list|add|disable INSTALLATION …",
    "inbox: cli=inbox list|add|remove|test INSTALLATION … | tui=inbox list|add|remove|test INSTALLATION …",
    "connection: cli=connection list|grant|revoke INSTALLATION … | tui=connection list|grant|revoke INSTALLATION …",
    'file: cli=file revoke INSTALLATION [OPERATION] | tui=file grant OPERATION "FILE"… | file save OPERATION "PATH" | file revoke INSTALLATION [OPERATION]',
  ])
  assert.match(cliAppUsage(), /chariox app create\|keygen\|manifest\|validate\|pack\|inspect\|install\|update\|list\|set\|/)
  assert.equal(tuiAppHelp().length, appCommandCatalog.filter((entry) => entry.tui).length)
})

test("file revoke reaches the kernel from the standalone CLI and TUI", async () => {
  for (const operation of [undefined, "operation-1"]) {
    const args = ["file", "revoke", "install-1", ...(operation ? [operation] : [])]
    const request = { RevokeAppFileGrants: { installation_id: "install-1",
      ...(operation ? { operation_id: operation } : {}) } }
    assert.deepEqual(await cliRequests(args), [request])
    assert.deepEqual(await tuiRequests(args), [request])
    assert.equal(sharedShellCommandForSlashCommand(`/app ${args.join(" ")}`), `app ${args.join(" ")}`)
  }
  assert.match(tuiAppHelp().join("\n"), /file revoke INSTALLATION \[OPERATION\]/)
})


test("publisher and file commands reach the local controller through slash routing", () => {
  for (const raw of ['/app publisher enroll "publisher.json"', '/app file grant operation-1 "file.txt"', '/app file save operation-1 "output.txt"']) {
    assert.equal(sharedShellCommandForSlashCommand(raw), null, raw)
    assert.equal(parseSlashCommand(raw)?.kind, "app")
  }
})

test("file grant and save still direct standalone CLI users to the TUI", async () => {
  for (const action of ["grant", "save"]) {
    await assert.rejects(runAppCommand(["app", "file", action, "operation-1", "file.txt"], {
      createClient: () => assert.fail("local file commands must not connect"),
      write: () => {},
    }), /app file runs in a Chariox terminal/)
  }
})


test("CLI open requires an explicit session while TUI open uses its attached session", async () => {
  await assert.rejects(runAppCommand(["app", "open", "install-1"], {
    createClient: () => assert.fail("missing session must not connect"),
    write: () => {},
  }), /pass --session/)
  const requests: unknown[] = []
  await handleAppSlashCommand({
    currentAppSessionId: () => "session-1",
    sendAppRequest: async (request) => { requests.push(request); return {} },
    appendNotice: () => {},
    flashFooter: () => {},
  }, { kind: "app", raw: "/app open install-1", args: ["open", "install-1"] }).catch(() => {})
  assert.deepEqual(requests, await cliRequests(["open", "install-1", "--session", "session-1"]))
})
