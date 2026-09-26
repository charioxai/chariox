import assert from "node:assert/strict"
import test from "node:test"
import { handleAppSlashCommand } from "./app-command-handler.js"

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
