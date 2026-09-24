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
