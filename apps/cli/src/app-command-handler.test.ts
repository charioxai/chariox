import assert from "node:assert/strict"
import test from "node:test"
import { parseSlashCommand, type ParsedSlashCommand } from "./commands.js"
import { appSlashArgs, handleAppSlashCommand } from "./app-command-handler.js"

test("App slash handler displays kernel installation state without session authority", async () => {
  const notices: string[] = []
  const requests: unknown[] = []
  await handleAppSlashCommand({
    sendAppRequest: async (request) => {
      requests.push(request)
      return { AppInstallation: { installation: {
        installation_id: "install-1", app_id: "dev.example.notes", generation: "9007199254740993",
        active_release: null, pending_generation: null, admission_paused: false,
      } } }
    },
    appendNotice: message => { notices.push(message) },
    flashFooter: message => assert.fail(message),
  }, { kind: "app", raw: "/app status install-1", args: ["status", "install-1"] })
  assert.deepEqual(requests, [{ GetAppInstallation: { installation_id: "install-1" } }])
  assert.match(notices[0]!, /install-1.*dev\.example\.notes.*generation 9007199254740993/)
})

test("App slash handler shows denials in footer without a success notice", async () => {
  const flashes: unknown[] = []
  await handleAppSlashCommand({
    sendAppRequest: async () => ({ AppRequestFailed: { code: "unauthorized" } }),
    appendNotice: message => assert.fail(message),
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
  }, { kind: "app", raw: "/app list", args: ["list"] })
  assert.deepEqual(flashes, [{ message: "This connection is not authorized to access Apps.", tone: "error" }])
})

test("/app file grant sends the chosen files' names and bytes, never their paths", async () => {
  const { mkdtemp, writeFile, rm } = await import("node:fs/promises")
  const { join } = await import("node:path")
  const { tmpdir } = await import("node:os")
  const dir = await mkdtemp(join(tmpdir(), "chariox-grant-"))
  try {
    const path = join(dir, "my notes.md")
    await writeFile(path, "# Notes")
    const requests: unknown[] = []
    const notices: string[] = []
    await handleAppSlashCommand({
      currentAppSessionId: () => "session-1",
      sendAppRequest: async (request) => {
        requests.push(request)
        return { AppFileGranted: { operation_id: "file-pick-1", files: 1 } }
      },
      appendNotice: message => { notices.push(message) },
      flashFooter: message => assert.fail(message),
    }, { kind: "app", raw: `/app file grant file-pick-1 "${path}"`, args: ["file", "grant", "file-pick-1", path] })
    assert.deepEqual(requests, [{ GrantAppFile: { session_id: "session-1", operation_id: "file-pick-1",
      files: [{ name: "my notes.md", contents_base64: Buffer.from("# Notes").toString("base64") }] } }])
    assert.equal(JSON.stringify(requests).includes(dir), false)
    assert.deepEqual(notices, ["Shared 1 file with the App."])
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("/app file revoke ends an installation's file requests through the shared App command", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const deps = {
    sendAppRequest: async (request: Record<string, unknown>) => {
      requests.push(request)
      return { AppFileGrantsRevoked: { installation_id: "docs", requests: 2, files: 1 } }
    },
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: (message: string) => assert.fail(message),
  }
  await handleAppSlashCommand(deps, { kind: "app", raw: "/app file revoke docs", args: ["file", "revoke", "docs"] })
  await handleAppSlashCommand(deps, { kind: "app", raw: "/app file revoke docs file-pick-1", args: ["file", "revoke", "docs", "file-pick-1"] })
  assert.deepEqual(requests, [
    { RevokeAppFileGrants: { installation_id: "docs" } },
    { RevokeAppFileGrants: { installation_id: "docs", operation_id: "file-pick-1" } },
  ])
  assert.equal(notices[0], "Revoked 2 file requests for docs; 1 granted file the App had not imported was dropped. "
    + "Files it already imported stay in its data.")
})

test("/app file save writes an offered file to a new path and never replaces one", async () => {
  const { mkdtemp, readFile, rm, writeFile } = await import("node:fs/promises")
  const { join } = await import("node:path")
  const { tmpdir } = await import("node:os")
  const dir = await mkdtemp(join(tmpdir(), "chariox-save-"))
  const notices: string[] = []
  try {
    const deps = {
      currentAppSessionId: () => "session-1",
      sendAppRequest: async (request: Record<string, unknown>) => {
        assert.deepEqual(request, { SaveAppFileExport: { session_id: "session-1", operation_id: "file-export-1" } })
        return { AppFileExport: { operation_id: "file-export-1", name: "plan.md", contents_base64: Buffer.from("# Plan").toString("base64") } }
      },
      appendNotice: (message: string) => notices.push(message),
      flashFooter: (message: string) => assert.fail(message),
    }
    const path = join(dir, "plan.md")
    await handleAppSlashCommand(deps, { kind: "app", raw: `/app file save file-export-1 "${path}"`, args: ["file", "save", "file-export-1", path] })
    assert.equal(await readFile(path, "utf8"), "# Plan")
    // The prompt is gone, so the notice says how to save the offer again.
    assert.match(notices[0] ?? "", /\/app file save file-export-1 "PATH"/)
    const existing = join(dir, "existing.md")
    await writeFile(existing, "keep")
    await assert.rejects(handleAppSlashCommand(deps, { kind: "app", raw: `/app file save file-export-1 "${existing}"`, args: ["file", "save", "file-export-1", existing] }))
    assert.equal(await readFile(existing, "utf8"), "keep")
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("/app arguments decode shell quoting but keep an inbox test payload exact", () => {
  assert.deepEqual(appSlashArgs('/app list --after "todo"'), ["list", "--after", "todo"])
  assert.deepEqual(appSlashArgs("/app status 'install 1'"), ["status", "install 1"])
  assert.deepEqual(appSlashArgs("/app"), [])
  assert.deepEqual(appSlashArgs('/app inbox test todo mail occ-1 {"title":"a  b"}'), ["inbox", "test", "todo", "mail", "occ-1", '{"title":"a  b"}'])
  assert.deepEqual(appSlashArgs(`/app inbox test todo mail occ-1 '{"title":"a  b"}'`), ["inbox", "test", "todo", "mail", "occ-1", '{"title":"a  b"}'])
  assert.deepEqual(appSlashArgs(`/app inbox test todo mail occ-1 '{"title":"it'\\''s"}'`), ["inbox", "test", "todo", "mail", "occ-1", `{"title":"it's"}`])
  assert.deepEqual(appSlashArgs('/app inbox test "todo" mail occ-1 "hi"'), ["inbox", "test", "todo", "mail", "occ-1", '"hi"'])
  // Quoted or escaped IDs with spaces end where the shell ends them.
  assert.deepEqual(appSlashArgs('/app inbox test todo "mail route" "occ 1" {"title":"x y"}'), ["inbox", "test", "todo", "mail route", "occ 1", '{"title":"x y"}'])
  assert.deepEqual(appSlashArgs(`/app inbox test todo 'mail route' occ\\ 1 '{"title":"x y"}'`), ["inbox", "test", "todo", "mail route", "occ 1", '{"title":"x y"}'])
  assert.deepEqual(appSlashArgs("/app inbox test todo mail occ-1"), ["inbox", "test", "todo", "mail", "occ-1"])
  // Not one quoted word: kept as typed, so the JSON check refuses it.
  assert.deepEqual(appSlashArgs(`/app inbox test todo mail occ-1 '{}' x`), ["inbox", "test", "todo", "mail", "occ-1", "'{}' x"])
  assert.throws(() => appSlashArgs('/app status "todo'), /unterminated quote/)
})

for (const [occurrence, payload] of [
  ["occ-1", '{"step":"request","message":"remote TUI T-05"}'],
  ["occ-1", `'{"step":"request","message":"remote TUI T-05"}'`],
  ['"occ 1"', '{"step":"request","message":"remote TUI T-05"}'],
  ['"occ 1"', `'{"step":"request","message":"remote TUI T-05"}'`],
] as const) {
  test(`/app inbox test sends the JSON payload exactly (${occurrence}, ${payload.startsWith("'") ? "single-quoted" : "bare"})`, async () => {
    const requests: unknown[] = []
    const notices: string[] = []
    const occurrenceId = occurrence.replaceAll('"', "")
    const raw = `/app inbox test todo mail ${occurrence} ${payload}`
    await handleAppSlashCommand({
      sendAppRequest: async (request) => {
        requests.push(request)
        return { AppInboxOccurrenceAccepted: { installation_id: "todo", route_id: "mail", occurrence_id: occurrenceId, duplicate: false } }
      },
      appendNotice: message => { notices.push(message) },
      flashFooter: message => assert.fail(message),
    }, parseSlashCommand(raw) as Extract<ParsedSlashCommand, { kind: "app" }>)
    assert.deepEqual(requests, [{ TestAppInboxRoute: {
      installation_id: "todo", route_id: "mail", occurrence_id: occurrenceId, payload: { step: "request", message: "remote TUI T-05" },
    } }])
    assert.deepEqual(notices, [`Occurrence ${occurrenceId} on mail accepted; the App receives it shortly.`])
  })
}

test("a copied next-page command pages from the decoded installation ID", async () => {
  const requests: unknown[] = []
  await handleAppSlashCommand({
    sendAppRequest: async (request) => {
      requests.push(request)
      return { AppInstallationsListed: { installations: [], next_cursor: null } }
    },
    appendNotice: () => {},
    flashFooter: message => assert.fail(message),
  }, parseSlashCommand('/app list --after "todo"') as Extract<ParsedSlashCommand, { kind: "app" }>)
  assert.deepEqual(requests, [{ ListAppInstallations: { after: "todo", limit: null } }])
})


test("/app open without an id shows only its own usage", async () => {
  const flashes: unknown[] = []
  await handleAppSlashCommand({
    currentAppSessionId: () => "session-1",
    sendAppRequest: async () => assert.fail("must not send an incomplete open"),
    appendNotice: message => assert.fail(message),
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
  }, { kind: "app", raw: "/app open", args: ["open"] })
  assert.deepEqual(flashes, [{ message: "usage: app open <installation-id> [--session <session-id>]", tone: "error" }])
})

test("/app open preserves the kernel's actionable transport error", async () => {
  const message = "This browser Environment belongs to Room apps-drill. Attach to that Room and use /app open todo, then /room view."
  await assert.rejects(handleAppSlashCommand({
    currentAppSessionId: () => "session-1",
    sendAppRequest: async request => {
      assert.deepEqual(request, { OpenAppView: { session_id: "session-1", installation_id: "todo" } })
      throw new Error(message)
    },
    appendNotice: notice => assert.fail(notice),
    flashFooter: notice => assert.fail(notice),
  }, { kind: "app", raw: "/app open todo", args: ["open", "todo"] }), { message })
})
