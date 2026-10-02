// Provider-free PTY fixture exercising the production shell and TUI composition.
import path from "node:path"
import { pathToFileURL } from "node:url"

const writeMarker = process.stdout.write.bind(process.stdout)
const root = process.env.HIDDENINPUT_BUILD_ROOT
const load = (name) => import(pathToFileURL(path.join(root, name)).href)
const [surface, command] = process.argv.slice(2)
const expected = process.env.HIDDENINPUT_FIXTURE_VALUE
let delivered = 0
let started = false
let lastRequest = "none"
const fakeSend = async (request) => {
  lastRequest = Object.keys(request)[0]
  if (request.ListProviderAccountProfiles) return { ProviderAccountProfilesListed: { profiles: [] } }
  if (request.SetCredentialSecret) {
    if (request.SetCredentialSecret.value !== expected) throw new Error("fixture value mismatch")
    delivered++
    return { CredentialSecretStored: { key: "synthetic" } }
  }
  if (request.SetProviderAccountCredential) {
    const data = request.SetProviderAccountCredential
    if (data.run === true) {
      started = true
      return { ProviderLoginStarted: { login: {
        login_id: "fixture-login", provider: "claude", account_profile: "default",
        login_kind: "terminal_setup_token", state: "running", login_url: null,
      } } }
    }
    if (data.value !== expected.trim()) throw new Error("fixture value mismatch")
    delivered++
    return { ProviderAccountCredentialStored: { provider: "claude", account_profile: "default", replaced: false } }
  }
  if (request.SendProviderLoginInput) {
    if (command === "run" && !started) throw new Error("setup flow did not start")
    if (Buffer.from(request.SendProviderLoginInput.data_base64, "base64").toString() !== `${expected}\r`) {
      throw new Error("fixture value mismatch")
    }
    delivered++
    return { ProviderLoginInputSent: { login_id: "fixture-login", byte_count: Buffer.byteLength(expected) + 1 } }
  }
  throw new Error("unexpected fixture request")
}

try {
  if (surface === "shell") {
    const { LocalIpcClient } = await load("packages/kernel-client/dist/ipc.js")
    LocalIpcClient.prototype.send = fakeSend
    const { runShellRepl } = await load(`${process.env.HIDDENINPUT_SHELL_DIST ?? "apps/shell/dist"}/shell.js`)
    const code = await runShellRepl({ kernelUrl: "ws://127.0.0.1:1/kernel" })
    if (command === "cancel" ? code !== 1 || delivered !== 0 : code !== 0 || delivered !== 1) throw new Error("shell did not deliver fixture input")
  } else {
    const { createCliRenderer, TextareaRenderable } = await import("@opentui/core")
    const { createCliSecretInput } = await load("apps/cli/dist/cli-secret-input.js")
    const { createCliCommandActionComposition } = await load("apps/cli/dist/cli-command-action-composition.js")
    const { createDefaultShellContext } = await load("packages/kernel-client/dist/shell-core.js")
    const { submitWorkspaceShellCommand } = await load("apps/cli/dist/workspace-shell-controller.js")
    const renderer = await createCliRenderer({ exitOnCtrlC: false, useConsole: false, useKittyKeyboard: {}, useThread: false })
    const prompt = new TextareaRenderable(renderer, { id: "normal-prompt", width: 70, height: 3 })
    renderer.root.add(prompt)
    prompt.focus()
    const hidden = createCliSecretInput(renderer)
    const notices = []
    const readSecret = (label) => {
      const result = hidden.readSecret(label)
      writeMarker("HIDDEN_READY\n")
      return result
    }
    const deps = new Proxy({
      options: { provider: "claude", clientId: "fixture" },
      client: { send: fakeSend },
      preferencesState: () => ({}), initialWorkspaceTarget: "/tmp", initialWorktreeTarget: "/tmp",
      isAttached: () => true, sessionState: () => ({ id: "fixture-session", agents: [], workflows: [] }),
      focusedAgentId: () => null, pendingWorkspaceTarget: () => "/tmp", pendingWorktreeTarget: () => "/tmp",
      flashFooter: (message) => notices.push(message), appendNotice: (message) => notices.push(message), readSecret,
    }, { get: (target, key) => key in target ? target[key] : () => undefined })
    const handlers = createCliCommandActionComposition(deps)
    try {
      if (command === "cancel") {
        try { await readSecret("Cancel test: "); throw new Error("cancellation did not reject") }
        catch (error) { if (error.message !== "secret input cancelled") throw error }
      } else if (command === "credential") await handlers.handleCredentialCommand({ kind: "credential", args: ["set", "synthetic"] })
      else if (command === "setup") await handlers.handleProviderCommand({ kind: "provider", value: "setup-token claude" })
      else if (command === "login" || command === "run") {
        if (command === "run") await handlers.handleProviderCommand({ kind: "provider", value: "setup-token claude --run" })
        await handlers.handleProviderCommand({ kind: "provider", value: "login-input fixture-login" })
      } else {
        let context = createDefaultShellContext({ workspace: "/tmp" })
        const entries = []
        await submitWorkspaceShellCommand("@ credential set synthetic", {
          client: deps.client, readSecret, workspaceShellContext: () => context,
          setWorkspaceShellContext: (value) => { context = value }, nextEntryId: () => 1,
          setWorkspaceShellEntries: (update) => { entries.push(...update([])) },
          sessionState: () => ({ id: "" }), selectedWorkflowId: () => null,
          rebuildTranscript: () => {}, flashFooter: deps.flashFooter,
        })
        if (JSON.stringify(entries).includes(expected)) throw new Error("embedded history leaked input")
      }
      if (delivered !== (command === "cancel" ? 0 : 1) || prompt.plainText !== "" || notices.some((value) => value.includes(expected))) {
        throw new Error("TUI delivery or history isolation failed")
      }
      if (renderer.currentFocusedRenderable !== prompt) throw new Error("focus was not restored")
    } finally {
      hidden.cancel()
      renderer.destroy()
    }
  }
  process.stdout.write("HIDDEN_PASS\n")
  process.exit(0)
} catch (error) {
  const known = ["fixture value mismatch", "shell did not deliver fixture input", "TUI delivery or history isolation failed", "focus was not restored", "unexpected fixture request"]
  process.stderr.write(`FIXTURE_REASON:${known.includes(error.message) ? error.message : error.name}:request=${lastRequest}\n`)
  // Never dump a request, assertion value, transcript or provider response.
  process.stderr.write("HIDDEN_FAIL\n")
  process.exit(1)
}
