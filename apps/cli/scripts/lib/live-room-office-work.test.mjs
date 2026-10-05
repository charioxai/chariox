import assert from "node:assert/strict"
import test from "node:test"
import { runRoomOfficeWork } from "./live-room-office-work.mjs"

// Model terminal attachment expiry at the two real idle boundaries without
// Docker, a provider, credentials, or a slow wall-clock sleep.
for (const boundary of ["office-installing", "office-mailing", "web-office-mailing", "provider-terminal-error",
  "provider-active-warning", "provider-old-error", "provider-completed-error"]) {
  test(`MP-08 MP-10 office prompt handling during ${boundary}`, async () => {
    const terminalError = ["provider-terminal-error", "provider-completed-error"].includes(boundary)
    const pending = ["provider-active-warning", "provider-old-error"].includes(boundary)
    const live = new Set()
    let next = 0
    let submitted = 0
    let loggedIn = false
    let typed = false
    let lastReport
    const contents = "Chariox office document\nPrepared through the graphical editor.\nGrüße from the Room.\n"
    const stop = new Error("second prompt reached kernel")
    const polling = new Error("provider remains pending")
    const requests = Object.fromEntries([
      "attachToSession", "detachFromSession", "submitPrompt", "listRoomEnvironmentActionHistory",
      "getSessionHistoryOutline", "getSessionState",
    ].map((name) => [`${name}Request`, (...args) => ({ name, args })]))
    const input = {
      requests, sessionId: "room", agentId: "agent", options: { provider: "codex", model: "fixture" },
      checkpoint: async ({ phase, office }) => {
        lastReport = office
        // The standard slice controller permits /workspace and Downloads,
        // not arbitrary files under the user's home directory.
        assert.ok(office.document.startsWith("/workspace/"), "office document must stay in the authorized upload workspace")
        if (phase === boundary) live.clear()
        if (boundary === "web-office-mailing" && phase === "office-mailing") {
          assert.equal(office.edit.localTuiObserved, false)
          assert.equal(office.edit.remoteTuiObserved, false)
          assert.equal(office.web.editor.matched, true)
        }
      },
      screenshot: async () => {},
      ...(boundary === "web-office-mailing" ? {
        deferTuiVerification: true,
        observeDesktop: async () => ({ matched: true }),
        waitForTuis: async () => { throw new Error("Web caller must not claim to observe TUIs") },
      } : { waitForTuis: async () => {} }),
      withTimeout: async (promise) => promise,
      waitFor: async (check, _timeout, message) => {
        // Production waiters retry read errors. A terminal failure must be a
        // truthy result, not an exception swallowed until the 300s deadline.
        const result = message === "office provider did not complete its task"
          ? await check().catch(() => false) : await check()
        if (pending && message === "office provider did not complete its task") {
          assert.equal(result, false, "an older error or active warning cannot terminate the current prompt")
          throw polling
        }
        assert.ok(result, "fixture did not reach the expected result")
        return result
      },
      client: { send: async ({ name, args }) => {
        if (name === "attachToSession") {
          const id = `attachment-${++next}`
          live.add(id)
          return { SessionAttached: { attachment: { id } } }
        }
        if (name === "detachFromSession") {
          live.delete(args[0])
          return { SessionDetached: {} }
        }
        if (name === "submitPrompt") {
          assert.ok(live.has(args[1]), "attachment was not found")
          submitted++
          if (submitted === 2) {
            assert.match(args[3], /call slice_browser_submit with its returned field_id exactly once/,
              "the mail prompt must request the explicit submit action required by attribution")
            throw stop
          }
          typed = !terminalError && !pending
          return { PromptSubmitted: { outcome: { Started: { prompt: { id: "prompt" } } } } }
        }
        if (name === "listRoomEnvironmentActionHistory") return { RoomEnvironmentActionHistoryListed: {
          page: { actions: typed ? [{ actor_id: "agent:agent", sequence: 1, action_id: "typed",
            kind: "keyboard_text", mode: "computer", state: "completed",
            arguments: { utf8_byte_count: Buffer.byteLength(contents) } }] : [] },
        } }
        if (name === "getSessionHistoryOutline") return { SessionHistoryOutline: { agents: [{ agent_id: "agent",
          turns: [{ prompt_id: boundary === "provider-old-error" ? "older-prompt" : "prompt", turn_id: "turn",
            lifecycle: boundary === "provider-completed-error" || !terminalError && !pending ? "completed" : "open",
            entries: terminalError || pending
              ? [{ entry: { kind: "provider_error", text: "synthetic-private-provider-detail-must-not-escape" } }] : [],
          }] }] } }
        if (name === "getSessionState") return { SessionState: {
          session: { agents: [{ id: "agent", is_processing: boundary === "provider-active-warning",
            state: terminalError || pending ? "Error" : "Idle" }] },
          agent_activity: { agent: { status: terminalError || pending ? "error" : "idle",
            last_error: "synthetic-private-provider-detail-must-not-escape" } },
        } }
        throw new Error(`unexpected fixture request ${name}`)
      } },
      officeRuntime: {
        containerName: "fixture",
        docker: async (args) => {
          let stdout = ""
          if (args.includes("node")) stdout = JSON.stringify({ browserRunning: true, insecureOriginException: false,
            sandboxDisabled: false, sandboxedRenderers: true, taskbarRunning: true, desktopSessionBus: true, editorSessionBus: true, editorDefaultSettings: true })
          else if (args.includes("/etc/xdg/openbox/menu.xml")) stdout = '<item label="Terminal emulator"><action name="Execute"><execute>x-terminal-emulator</execute>'
          else if (args.some((arg) => arg.includes("CHARIOX_SLICE_DISPLAY"))) stdout = ":99"
          else if (args.some((arg) => arg.includes("WM_CLASS"))) stdout = "Mousepad"
          else if (args.includes("cat")) stdout = contents
          return { stdout }
        },
        sliceScreen: async ([action]) => {
          if (action === "browser-submit") loggedIn = true
          return loggedIn ? "CHARIOX_FIXTURE_INBOX" : "Fixture mail login"
        },
        runCommandWithStdin: async () => ({ code: 0 }),
      },
    }
    await assert.rejects(runRoomOfficeWork(input), (error) => terminalError
      ? error.message === "office provider failed before the required physical result"
      : error === (pending ? polling : stop))
    assert.equal(submitted, terminalError || pending ? 1 : 2)
    assert.equal(lastReport.prompts.length, 1)
    assert.equal(lastReport.prompts[0].promptId, "prompt")
    assert.equal(lastReport.prompts[0].admission, "Started")
    if (!terminalError && !pending) assert.equal(lastReport.prompts[0].turnId, "turn")
    assert.equal(JSON.stringify(lastReport).includes("synthetic-private-provider-detail-must-not-escape"), false)
    assert.equal(live.size, 0, "prompt attachments must be released even when submission fails")
  })
}
