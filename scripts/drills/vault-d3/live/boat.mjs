// The fixture Boat: restart the fixture server (exact pid), read its server-side logs, and pipe a fixture needle into the scanner without it
// ever touching argv, env or this process's output. Needs BOAT_HOST and BOAT_KEY (an ssh key file path; never read here).
import { execFileSync, spawn } from "node:child_process"
import { createReadStream } from "node:fs"

const host = process.env.BOAT_HOST ?? "user@2001:41d0:1008:a1f:100::20"
const key = process.env.BOAT_KEY ?? "/root/.ssh/boat_builders_ed25519"
const sshArgs = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-i", key, host]
export const boat = command => execFileSync("ssh", [...sshArgs, command], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 })

export function restartFixtures({ name = "2001-41d0-1008-a1f-100--20.sslip.io", qrFrom = "/home/user/qr/node_modules" } = {}) {
  boat(`set -e; if [ -f ~/fixtures.pid ]; then kill $(cat ~/fixtures.pid) 2>/dev/null || true; sleep 1; fi; rm -rf ~/fixture-state; mkdir -m 700 ~/fixture-state; cd ~/d3
    QR_FROM=${qrFrom} nohup node scripts/drills/vault-d3/fixtures/serve.mjs --state-dir /home/user/fixture-state --host :: --public-host ${name} --alt-host alt.${name} --base-port 8441 --tls-key /home/user/certs/privkey.pem --tls-cert /home/user/certs/fullchain.pem >/home/user/fixtures.log 2>&1 &
    echo $! > ~/fixtures.pid; sleep 2; cat ~/fixtures.log | head -c 600 >/dev/null`)
  return JSON.parse(boat("cat ~/fixture-state/origins.json"))
}
export const stopFixtures = () => boat("if [ -f ~/fixtures.pid ]; then kill $(cat ~/fixtures.pid) 2>/dev/null || true; rm -f ~/fixtures.pid; fi; rm -rf ~/fixture-state; true")
export async function fixtureState(origin) { const r = await fetch(origin + "/state"); return r.json() }

/** Run never-seen with a needle file piped to its stdin: {boat: "/path on the Boat"} or {file: "/local/path"}. Returns {status, stdout}: counts only. */
export function scanWithNeedle({ source, scanner, args }) {
  return new Promise(resolve => {
    const scan = spawn(process.execPath, [scanner, "--needle-stdin", ...args], { stdio: ["pipe", "pipe", "pipe"] })
    if (source.boat) { const ssh = spawn("ssh", [...sshArgs, `cat ${source.boat}`], { stdio: ["ignore", "pipe", "ignore"] }); ssh.stdout.pipe(scan.stdin) }
    else createReadStream(source.file).pipe(scan.stdin)
    let out = ""
    scan.stdout.on("data", c => { out += c }); scan.stderr.on("data", c => { out += c })
    scan.on("close", status => resolve({ status, stdout: out.trim() }))
  })
}
