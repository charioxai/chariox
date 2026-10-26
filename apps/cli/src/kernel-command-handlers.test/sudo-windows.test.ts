import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"
import { parseSlashCommand } from "../commands.js"
import { sudoWindowLine } from "../sudo-window-line.js"
import { assert, handleKernelSlashCommand, makeSession, test } from "../kernel-command-handlers.test-support.js"

const window = (id: string, agent = "agent-1"): KernelSudoTurn => ({
  entry_id: id, session_id: "session-1", agent_id: agent, owner_user_id: "local", terminal_id: "t",
  prompt_id: "p", provider_run_id: "r", task_id: id, duration_minutes: 60,
  expires_at_ms: Date.parse("2026-10-07T14:05:00Z"), revision: 3, warning_sent: false,
})

test("MP-08/MP-10/MP-11 A04 /sudo window controls parse; any other /sudo text stays a prompt", () => {
  assert.deepEqual(parseSlashCommand("/sudo extend"), { kind: "kernel", raw: "/sudo extend", args: ["sudo", "extend"] })
  assert.deepEqual(parseSlashCommand("/sudo revoke sudo:00ff"), { kind: "kernel", raw: "/sudo revoke sudo:00ff", args: ["sudo", "revoke", "sudo:00ff"] })
  assert.deepEqual(parseSlashCommand("/sudo status"), { kind: "kernel", raw: "/sudo status", args: ["sudo", "status"] })
  for (const prompt of ["/sudo extend the database schema", "/sudo deploy", "/sudo status of prod please"]) {
    assert.equal(parseSlashCommand(prompt), null, prompt)
  }
})

test("MP-08/MP-10/MP-11 A04 the status line reads the kernel deadline, not a client countdown", () => {
  const now = Date.parse("2026-10-07T12:30:00Z")
  assert.equal(sudoWindowLine(window("sudo:01"), now, "builder"), "sudo · builder · 1h 35m left (until 14:05 UTC) · sudo:01")
  assert.match(sudoWindowLine({ ...window("sudo:01"), warning_sent: true }, Date.parse("2026-10-07T13:58:00Z")), /7m left .* expiring soon/)
})

test("MP-08/MP-10/MP-11 A04 /sudo extend and revoke target the named or only live window", async () => {
  const calls: string[] = []
  const flashes: string[] = []
  const notices: string[] = []
  let windows = [window("sudo:01")]
  const deps = {
    isAttached: () => true,
    sessionState: () => makeSession(),
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: (message: string) => { flashes.push(message) },
    transitionToNoSession: () => {},
    sudoWindows: () => windows,
    revokeKernelAccessGrant: async (id: string | null) => { calls.push(`revoke ${id}`); return 1 },
    extendKernelSudo: async (id: string, revision: number) => { calls.push(`extend ${id}@${revision}`); return { ...window(id), revision: revision + 1 } },
  }
  await handleKernelSlashCommand(deps, { kind: "kernel", raw: "/sudo extend", args: ["sudo", "extend"] })
  await handleKernelSlashCommand(deps, { kind: "kernel", raw: "/sudo status", args: ["sudo", "status"] })
  windows = [window("sudo:01"), window("sudo:02", "agent-2")]
  await handleKernelSlashCommand(deps, { kind: "kernel", raw: "/sudo revoke", args: ["sudo", "revoke"] })
  await handleKernelSlashCommand(deps, { kind: "kernel", raw: "/sudo revoke sudo:02", args: ["sudo", "revoke", "sudo:02"] })
  assert.deepEqual(calls, ["extend sudo:01@3", "revoke sudo:02"])
  assert.match(flashes.join("\n"), /several sudo windows are live/)
  assert.match(notices.join("\n"), /sudo:01 extended/)
  assert.match(notices.join("\n"), /sudo · agent-1 · .* left/)
})
