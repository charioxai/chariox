// MP-08 / MP-10 / MP-11: receiver-local CLIENT authority, never credential transfer.
import { mkdir, writeFile, access } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { approveDevDeviceLogin } from "./live-hosted-cloud-relay-drill-helpers.mjs"

export async function prepareHostedRemoteTerminal({
  remoteRoot, remoteCliRepo, apiUrl, ownerAccountSlug, ownerAccountId,
  runSsh, shellQuote, sshArgs, spawnProcess, terminateChild,
  approve = approveDevDeviceLogin,
}) {
  if (!/^\/var\/tmp\/chariox-kauthval-hosted-remote-cli-\d+-\d+$/.test(remoteRoot)) {
    throw new Error("receiver state must be a lane-owned disposable root")
  }
  if (!apiUrl || !ownerAccountSlug || !ownerAccountId) throw new Error("receiver login requires the enrolled owner's account")
  const environment = [
    "export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH",
    `export HOME=${shellQuote(remoteRoot + "/home")}`,
    `export CHARIOX_HOME=${shellQuote(remoteRoot + "/home/.chariox")}`,
    `export XDG_CONFIG_HOME=${shellQuote(remoteRoot + "/home/.config")}`,
    `export XDG_STATE_HOME=${shellQuote(remoteRoot + "/home/.local/state")}`,
    `export TMPDIR=${shellQuote(remoteRoot + "/tmp")}`,
    "export NODE_OPTIONS=--max-old-space-size=4096",
  ].join("; ")
  const script = path.posix.join(remoteCliRepo, "apps/cli/scripts/lib/hosted-cloud-remote-terminal-login.mjs")
  const command = mode => `${environment}; node ${shellQuote(script)} ${mode} ${shellQuote(remoteRoot)} ${shellQuote(apiUrl)} ${shellQuote(ownerAccountId)}`
  const setup = await runSsh(`umask 077; mkdir -p ${shellQuote(remoteRoot + "/tmp")}`)
  if (setup.code !== 0) throw new Error("receiver state initialization failed")
  let login
  const cleanup = async () => {
    const abort = await runSsh(`touch ${shellQuote(remoteRoot + "/abort-login")}`)
    if (abort.code !== 0) throw new Error("receiver login cancellation failed")
    await terminateChild(login)
    const logout = await runSsh(command("logout"))
    if (logout.code !== 0) throw new Error("receiver CLIENT logout was not acknowledged; isolated state retained")
    const removed = await runSsh(`rm -rf ${shellQuote(remoteRoot)}`)
    if (removed.code !== 0) throw new Error("receiver state cleanup failed")
  }
  try {
    login = spawnProcess("ssh", sshArgs(command("login")), { name: "remote-client-login", env: process.env })
    const deadline = Date.now() + 10 * 60 * 1000
    let approved = false
    while (Date.now() < deadline) {
      // Only the deliberately public challenge projection is read remotely.
      const challenge = await runSsh(`cat ${shellQuote(remoteRoot + "/verification.json")}`)
      if (!approved && challenge.code === 0) {
        const value = JSON.parse(challenge.stdout)
        await approve({ role: "remote-receiver", userCode: value.userCode, verificationUrl: value.verificationUrl, accountSlug: ownerAccountSlug })
        approved = true
      }
      const ready = await runSsh(`cat ${shellQuote(remoteRoot + "/login-ready")}`)
      if (ready.code === 0) return { environment, cleanup, publicClient: JSON.parse(ready.stdout) }
      if (login.exitCode != null || login.signalCode != null) throw new Error("receiver CLIENT login failed before bootstrap")
      await new Promise(resolve => setTimeout(resolve, 1000))
    }
    throw new Error("receiver CLIENT device login timed out")
  } catch (error) {
    await cleanup()
    throw error
  }
}

// Run on the receiving machine using the SAME default store/identity paths as
// bootstrapCliRuntime. Only public challenges and completion markers escape.
export async function runRemoteTerminalLogin(mode, root, apiUrl, accountId) {
  const { CloudClient } = await import("../../dist/cloud-client.js")
  const client = new CloudClient()
  let cancellation
  try {
    if (mode === "login") {
      cancellation = setInterval(() => {
        access(path.join(root, "abort-login")).then(() => { client.stop(); process.exit(1) }, () => {})
      }, 500)
      await mkdir(root, { recursive: true, mode: 0o700 })
      const profile = await client.login(apiUrl, async ({ verificationUrl, userCode }) => {
        await writeFile(path.join(root, "verification.json"), JSON.stringify({ verificationUrl, userCode }), { mode: 0o600 })
      }, accountId)
      // Only the public, key-bound CLIENT identity leaves this machine.
      await writeFile(path.join(root, "login-ready"), JSON.stringify({accountId: profile.accountId,
        clientId: profile.clientId, publicKeyThumbprint: profile.clientId.slice("cli:".length)}), { mode: 0o600 })
    } else if (mode === "logout") {
      await client.logout()
    } else throw new Error("unknown receiver login operation")
  } finally { clearInterval(cancellation); client.stop() }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  runRemoteTerminalLogin(...process.argv.slice(2)).catch(() => {
    console.error("MP-08 / MP-10 / MP-11 receiver CLIENT operation failed")
    process.exitCode = 1
  })
}
