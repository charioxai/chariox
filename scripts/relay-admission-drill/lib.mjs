// Test-only helpers for the G09/G10 relay admission drills: IPC clients for the drill kernels, copies of their
// state directories (the senders) and the test-only peer process (`admission_drill_peer`, built with `cargo test --no-run`).
// ADMISSION_DRILL_STATE: scratch directory for sender copies and ids; ADMISSION_DRILL_EVIDENCE: where receipts are written.
import assert from 'node:assert/strict'
import { readFileSync, mkdirSync, cpSync, rmSync, appendFileSync } from 'node:fs'
import { spawn } from 'node:child_process'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'
const repo = fileURLToPath(new URL('../..', import.meta.url)).replace(/\/$/, '')
export const { LocalIpcClient } = await import(`${repo}/packages/kernel-client/dist/ipc.js`)
export const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
export const EVID = process.env.ADMISSION_DRILL_EVIDENCE
export const STATE = process.env.ADMISSION_DRILL_STATE
assert.ok(EVID && STATE, 'set ADMISSION_DRILL_EVIDENCE and ADMISSION_DRILL_STATE')
export const sleep = ms => new Promise(r => setTimeout(r, ms))
const SKIP = /TOKEN|SECRET|PASS|CREDENTIAL|KEY/i

export function envOf(pid) {
  const out = {}
  for (const kv of readFileSync(`/proc/${pid}/environ`, 'utf8').split('\0')) {
    const i = kv.indexOf('='); if (i < 0) continue
    const k = kv.slice(0, i); if (SKIP.test(k)) continue
    if (/^(HOME|TMPDIR|PATH|XDG_[A-Z_]+|CHARIOX_[A-Z_]+)$/.test(k)) out[k] = kv.slice(i + 1)
  }
  return out
}
export const client = (pid) => { const env = envOf(pid); return new LocalIpcClient(`ws://127.0.0.1:${env.CHARIOX_KERNEL_PORT}`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 15000 }) }
export const unwrap = (r, k) => { if (r.Error) throw new Error(JSON.stringify(r.Error).slice(0, 300)); assert.ok(r[k], `missing ${k}: ${Object.keys(r)}`); return r[k] }

// copy a kernel's identity/config dirs so the peer process never touches the live kernel's files
export function senderCopy(name, pid) {
  const env = envOf(pid); const root = env.HOME.replace(/\/user$/, ''); const dst = `${STATE}/senders/${name}`
  rmSync(dst, { recursive: true, force: true }); mkdirSync(dst, { recursive: true, mode: 0o700 })
  for (const d of ['kernel', 'config', 'state', 'data', 'cache']) cpSync(`${root}/${d}`, `${dst}/${d}`, { recursive: true })
  for (const d of ['user', 'tmp', 'logs']) mkdirSync(`${dst}/${d}`, { recursive: true })
  const mapped = Object.fromEntries(Object.entries(env).map(([k, v]) => [k, v.startsWith(root) ? dst + v.slice(root.length) : v]))
  delete mapped.CHARIOX_DAEMON_SOCKET
  return { name, env: mapped }
}

export class Peer {
  constructor(bin, senders) { this.bin = bin; this.senders = senders; this.pending = new Map(); this.n = 0 }
  async start() {
    this.proc = spawn(this.bin, ['--ignored', '--nocapture', '--exact', 'transport::relay_client::admission_drill::admission_drill_peer'], {
      cwd: repo, env: { PATH: process.env.PATH, RUST_MIN_STACK: '16777216', CHARIOX_ADMISSION_DRILL_SENDERS: JSON.stringify(this.senders) }, stdio: ['pipe', 'pipe', 'pipe'] })
    this.stderr = ''; this.proc.stderr.on('data', d => { this.stderr = (this.stderr + d).slice(-4000) })
    const rl = readline.createInterface({ input: this.proc.stdout })
    let ready; const readyP = new Promise(r => { ready = r })
    rl.on('line', line => {
      if (line.startsWith('READY ')) return ready(line)
      if (line.startsWith('RECEIPT ')) { const r = JSON.parse(line.slice(8)); this.pending.get(r.id)?.(r); this.pending.delete(r.id) }
    })
    this.proc.on('exit', code => { this.exited = code; ready(null) })
    const r = await Promise.race([readyP, sleep(120000).then(() => null)])
    assert.ok(r, 'peer did not become ready: ' + this.stderr.slice(-600))
  }
  send(sender, targetKernelId, request, timeoutMs = 90000) {
    const id = `c${++this.n}`
    const p = new Promise(res => { this.pending.set(id, res); setTimeout(() => { if (this.pending.delete(id)) res({ id, outcome: { kind: 'driver_timeout' } }) }, timeoutMs) })
    this.proc.stdin.write(JSON.stringify({ id, sender, target_kernel_id: targetKernelId, request }) + '\n')
    return p
  }
  stop() { if (this.proc && this.proc.exitCode === null && this.proc.pid > 1) this.proc.kill('SIGTERM') }
}

export const log = (file, obj) => appendFileSync(`${EVID}/${file}`, JSON.stringify({ at: new Date().toISOString(), ...obj }) + '\n')
