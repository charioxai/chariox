// MP-07 / MP-08 / MP-11: lane-owned localhost sshd fixture; synthetic ephemeral keys.
import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { once } from "node:events"
const quote = s => `'${s.replaceAll("'", `'"'"'`)}'`
export async function availablePort() {
  const s = createServer(); await new Promise(r => s.listen(0, "127.0.0.1", r))
  const p = s.address().port; await new Promise(r => s.close(r)); return p
}
function quiet(program, args) {
  const result = spawnSync(program, args, { stdio: "ignore" })
  if (result.status !== 0) throw new Error("SSH fixture prerequisite failed")
}
export async function stopOwned(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing unsafe drill PID")
  const stopped = once(child, "close")
  child.kill("SIGTERM")
  await stopped
}
export async function sshTarget(t, { beforeCleanup = async () => {} } = {}) {
  const base = process.env.CHARIOX_BYOM_TEST_STATE ?? join(tmpdir(), "chariox-byom-tests")
  await mkdir(base, { recursive: true, mode: 0o700 })
  const scratch = await mkdtemp(join(base, "ssh-drill-"))
  const home = join(scratch, "target-home"), client = join(scratch, "client-home"), bin = join(scratch, "target-bin")
  for (const p of [home, client, bin, join(client, ".ssh")]) await mkdir(p, { mode: 0o700 })
  let daemon
  const envNames = ["HOME", "PATH"]
  const saved = Object.fromEntries(envNames.map(k => [k, process.env[k]]))
  t.after(async () => {
    await beforeCleanup()
    for (const k of envNames) { if (saved[k] === undefined) delete process.env[k]; else process.env[k] = saved[k] }
    if (daemon) await stopOwned(daemon)
    // Inventory: only this fixture's generated SSH identities, public pins, fixture bytes and home state.
    // No owner key, linked account, reviewer state or protected shared path lives under scratch.
    await rm(scratch, { recursive: true, force: true })
    await assert.rejects(readFile(join(scratch, "host-key")), { code: "ENOENT" })
  })
  for (const name of ["host-key", "client-key"]) quiet("ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", join(scratch, name)])
  await copyFile(join(scratch, "client-key.pub"), join(scratch, "authorized_keys"))
  const port = await availablePort()
  await writeFile(join(client, ".ssh/config"), `Host byom-local\n  HostName 127.0.0.1\n  User root\n  Port ${port}\n  IdentityFile ${join(scratch, "client-key")}\n  IdentitiesOnly yes\n  UserKnownHostsFile ${join(scratch, "known_hosts")}\n  StrictHostKeyChecking yes\n`, { mode: 0o600 })
  // OpenSSH reads the account home, not env HOME; a lane-owned ssh wrapper selects this fixture config.
  const localBin = join(scratch, "local-bin"); await mkdir(localBin)
  await writeFile(join(localBin, "ssh"), `#!/bin/sh\nexec /usr/bin/ssh -F ${quote(join(client, ".ssh/config"))} "$@"\n`, { mode: 0o700 })
  const pub = (await readFile(join(scratch, "host-key.pub"), "utf8")).trim().split(" ").slice(0, 2).join(" ")
  await writeFile(join(scratch, "known_hosts"), `[127.0.0.1]:${port} ${pub}\n`, { mode: 0o600 })
  const targetPath = `${bin}:${saved.PATH}`
  // This supplementary fixture models an already-persistent user manager.
  await writeFile(join(bin, "loginctl"), '#!/bin/sh\n[ "$1" = show-user ] || exit 2\nprintf "yes\\n"\n', { mode: 0o700 })
  const forced = join(scratch, "forced.sh")
  await writeFile(forced, `#!/bin/sh\nexport HOME=${quote(home)}\nexport PATH=${quote(targetPath)}\nexec /bin/sh -c "$SSH_ORIGINAL_COMMAND"\n`, { mode: 0o700 })
  await writeFile(join(bin, "systemctl"), `#!/usr/bin/env node
import fs from "node:fs"
import path from "node:path"
const args = process.argv.slice(2)
if (args.shift() !== "--user") process.exit(2)
if (!args.every(a => !a.endsWith(".service") || /^chariox-ssh-byom-(one|two|tampered)\\.service$/.test(a))) process.exit(3)
fs.appendFileSync(path.join(process.env.HOME,"fixture-service-calls"),JSON.stringify(args)+"\\n")
if (args[0] === "show") {
 const p = path.join(process.env.HOME,".config/systemd/user",args[1]); const exists=fs.existsSync(p)
 console.log("LoadState="+(exists?"loaded":"not-found")+"\\nFragmentPath="+(exists?p:"")+"\\nDropInPaths=\\nActiveState=inactive\\nUnitFileState=disabled")
} else if (args[0] === "daemon-reload" && fs.existsSync(path.join(process.env.HOME,"fixture-fail-reload"))) {
 fs.unlinkSync(path.join(process.env.HOME,"fixture-fail-reload")); process.exit(5)
} else if (!["daemon-reload","disable"].includes(args[0])) process.exit(4)
`, { mode: 0o700 })
  // Node treats an extensionless executable as CommonJS: use require for the test manager.
  const ctl = join(bin, "systemctl")
  let ctlText = await readFile(ctl, "utf8"); ctlText = ctlText.replace('import fs from "node:fs"', 'const fs = require("node:fs")').replace('import path from "node:path"', 'const path = require("node:path")'); await writeFile(ctl, ctlText); await chmod(ctl, 0o700)
  const config = join(scratch, "sshd_config")
  await writeFile(config, `Port ${port}\nListenAddress 127.0.0.1\nHostKey ${join(scratch, "host-key")}\nPidFile ${join(scratch, "sshd.pid")}\nAuthorizedKeysFile ${join(scratch, "authorized_keys")}\nStrictModes yes\nPermitRootLogin yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM yes\nAllowUsers root\nForceCommand ${forced}\nAllowTcpForwarding no\nX11Forwarding no\nPermitTTY no\nLogLevel VERBOSE\n`)
  quiet("/usr/sbin/sshd", ["-t", "-f", config])
  daemon = spawn("/usr/sbin/sshd", ["-D", "-e", "-f", config], { stdio: ["ignore", "ignore", "pipe"] })
  let daemonReason = "unknown fixture startup failure"
  daemon.stderr.on("data", chunk => {
    const s = chunk.toString()
    for (const [pattern, reason] of [["Missing privilege separation directory", "missing privilege separation directory"], ["Address already in use", "fixture SSH port collision"], ["account is locked", "fixture root account locked"], ["bad ownership or modes", "fixture authentication ownership failed"]]) if (s.includes(pattern)) daemonReason = reason
  })
  daemon.once("error", () => {})
  process.env.PATH = `${localBin}:${saved.PATH}`
  // Wait with bounded connection attempts; do not signal unrelated sshd processes.
  let ready = false, clientReason = "unknown SSH failure"
  for (let i = 0; i < 40; i++) {
    const r = spawnSync(join(localBin, "ssh"), ["-oBatchMode=yes", "byom-local", "true"], { stdio: ["ignore", "ignore", "pipe"] })
    for (const reason of ["Permission denied", "Host key verification failed", "Bad configuration option", "Bad owner or permissions", "Connection refused", "Connection closed", "Connection reset", "No such file", "Too many authentication failures"]) if (r.stderr?.toString().includes(reason)) clientReason = reason
    if (r.status === 0) { ready = true; break }
    await new Promise(r => setTimeout(r, 50))
  }
  assert.equal(ready, true, `fixture sshd must authenticate with its pinned host key: ${daemonReason}; ${clientReason}`)
  return { scratch, home, client, bin, targetPath, saved }
}
