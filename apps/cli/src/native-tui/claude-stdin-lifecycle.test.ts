// MP-08 / MP-10 / MP-11: restoring native input must let the real process exit.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import test from "node:test"
import { signalNativeChild } from "./native-child-signals.js"

test("rendered Claude releases stdin after its official provider exits", async () => {
  const module = new URL("./claude-remote-rendered-io.js", import.meta.url).href
  const child = spawn(process.execPath, ["--input-type=module", "-e", `
    import { forwardClaudeRemoteRenderedStdin } from ${JSON.stringify(module)};
    const restore = forwardClaudeRemoteRenderedStdin({client: {}, sessionId: 's', attachmentId: 'a',
      agentId: 'agent', providerRunId: 'run', worktree: '/', inlineLocalAttachments: false, debug: () => {}});
    restore();
    process.stdout.write('restored');
  `], { stdio: ["pipe", "pipe", "pipe"] })
  const exit = once(child, "exit")
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    await once(child.stdout!, "data")
    const result = await Promise.race([
      exit,
      new Promise<null>((resolve) => { deadline = setTimeout(() => resolve(null), 1_000) }),
    ])
    assert(result, "native wrapper retains a resumed stdin after cleanup")
    assert.equal(result[0], 0)
  } finally {
    clearTimeout(deadline)
    child.stdin!.destroy()
    if (child.exitCode === null && child.signalCode === null) signalNativeChild(child, "SIGKILL")
    await exit
  }
})
