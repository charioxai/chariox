#!/usr/bin/env node
// Row 551 kernel-side cells on a drill kernel (its own CHARIOX_HOME, ports and systemd unit; never a live kernel):
//  K1 the owner's RestartKernel is answered `KernelRestarting` before the kernel stops;
//  K2 the PID stays (hot reload), the program starts again (a hot_reload.receive log line);
//  K3 a Vault unlocked for 1 minute is still unlocked right after the restart and locks at its own deadline (the restart does not extend it);
//  K4 a hot reload that cannot be prepared (the installed program cannot be verified) is refused with its reason and the kernel keeps running.
// env: DRILL_HOME (CHARIOX_HOME), DRILL_PORT, DRILL_UNIT, DRILL_BIN (the installed program), KERNEL_CLIENT (path of packages/kernel-client/dist/ipc.js)
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { chmodSync, readFileSync, readdirSync } from "node:fs"
import path from "node:path"
const { DRILL_HOME, DRILL_PORT, DRILL_UNIT, DRILL_BIN, KERNEL_CLIENT } = process.env
for (const value of [DRILL_HOME, DRILL_PORT, DRILL_UNIT, DRILL_BIN, KERNEL_CLIENT]) assert(value, "DRILL_HOME, DRILL_PORT, DRILL_UNIT, DRILL_BIN and KERNEL_CLIENT are required")
const { LocalIpcClient } = await import(KERNEL_CLIENT)
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const mainPid = () => Number(execFileSync("systemctl", ["show", DRILL_UNIT, "-p", "MainPID", "--value"], { encoding: "utf8" }).trim())
const logs = path.join(DRILL_HOME, "..", "logs")
const receives = () => { let n = 0; for (const f of readdirSync(logs).filter(f => f.endsWith(".ndjson"))) for (const l of readFileSync(path.join(logs, f), "utf8").split("\n")) if (l.includes('"hot_reload.receive"')) n++; return n }
const connect = async () => {
  const client = new LocalIpcClient(`ws://127.0.0.1:${DRILL_PORT}/kernel`, { localAuthEnvironment: { ...process.env, CHARIOX_HOME: DRILL_HOME }, controlRequestRetryDeadlineMs: 2000 })
  const rpc = async request => { const r = await client.send(request); if (r.Error) throw Error(JSON.stringify(r.Error).slice(0, 400)); return r }
  await rpc({ RelayStatus: null })
  return { client, rpc }
}
const results = {}
let { rpc } = await connect()
const vault = async () => (await rpc({ GetCredentialVaultStatus: {} })).CredentialVaultStatus.status
// K3 set-up: a Vault unlocked for 10 minutes
try { await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: "drill-passphrase-551" } } }) } catch (error) { if (!/exist|already/i.test(String(error))) throw error }
await rpc({ LockCredentialVault: {} }) // a clean start: an earlier, longer unlock would otherwise keep its deadline
await rpc({ UnlockCredentialVault: { passphrase: "drill-passphrase-551", duration: { kind: "minutes", minutes: 1 }, surface: "settings" } })
const unlockedAt = Date.now()
const before = await vault()
const pid0 = mainPid(), receives0 = receives()
// K1/K2/K3
const askedAt = Date.now()
const reply = await rpc({ RestartKernel: {} })
results.K1 = { answered: Object.keys(reply)[0], answeredAfterMs: Date.now() - askedAt }
let back = null
// back = the NEW program answers: its hot_reload.receive line is written and a request is served
for (let n = 0; n < 240 && back === null; n++) { await sleep(250); if (receives() <= receives0) continue; try { ({ rpc } = await connect()); back = (Date.now() - askedAt) / 1000 } catch {} }
assert(back !== null, "the kernel came back")
await sleep(1000)
const after = await vault()
results.K2 = { pid_same: mainPid() === pid0, pid: [pid0, mainPid()], hot_reload_receive_lines: [receives0, receives()], back_after_s: back }
results.K3 = { right_after_restart: after.unlocked }
// the deadline is the one set at the unlock: 65 s after it, the Vault is locked again
await sleep(Math.max(0, unlockedAt + 65_000 - Date.now()))
results.K3.after_its_deadline = (await vault()).unlocked
// K4: the installed program cannot be verified: the request is refused with the reason, the kernel keeps running
const mode = readFileSync(DRILL_BIN).length && (await import("node:fs")).statSync(DRILL_BIN).mode
chmodSync(DRILL_BIN, 0o644)
try {
  const pid1 = mainPid(), receives1 = receives()
  let refusal = null
  try { await rpc({ RestartKernel: {} }) } catch (error) { refusal = String(error).replace(/\s+/g, " ").slice(0, 300) }
  await sleep(1500)
  results.K4 = { refusal, pid_same: mainPid() === pid1, no_new_reload: receives() === receives1, still_answers: Boolean((await rpc({ RelayStatus: null })).RelayStatus) }
} finally { chmodSync(DRILL_BIN, mode & 0o777) }
console.log(JSON.stringify(results, null, 1))
assert.equal(results.K1.answered, "KernelRestarting")
assert(results.K3.right_after_restart === true && results.K3.after_its_deadline === false, "the Vault survived the restart and kept its deadline")
assert(results.K2.pid_same && results.K2.hot_reload_receive_lines[1] === results.K2.hot_reload_receive_lines[0] + 1, "same PID and one new hot reload")
assert(results.K4.refusal && /hot reload refused/.test(results.K4.refusal) && results.K4.pid_same && results.K4.no_new_reload && results.K4.still_answers, "a hot reload that cannot be prepared is refused and the kernel keeps running")
