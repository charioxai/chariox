// MP-08 / MP-09 / MP-10 / MP-11: a real self-hosted TOTP service for the capture drill.
// Gitea's two-factor enrolment shows both a manual setup key and a QR code, and its
// login refuses a reused passcode. The binary is downloaded outside the repository.
import { execFileSync, spawn } from 'node:child_process'
import { chmodSync, copyFileSync, mkdirSync, rmSync, writeFileSync, readFileSync } from 'node:fs'
import { randomBytes } from 'node:crypto'
import net from 'node:net'
import path from 'node:path'

const run = (command, args, options = {}) => execFileSync('/usr/bin/setpriv', ['--reuid=ubuntu', '--regid=ubuntu', '--clear-groups', command, ...args],
  { encoding: 'utf8', env: { PATH: '/usr/bin:/bin', HOME: options.home }, ...options })
const freePort = () => new Promise((resolve, reject) => { const server = net.createServer(); server.on('error', reject); server.listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => resolve(port)) }) })

/** Starts Gitea as the unprivileged `ubuntu` user under `home`. The caller supplies the password
 *  through `password`; it never appears in a command line (the account is created from a file). */
export async function startGitea({ binary, home, user, password }) {
  if (!path.isAbsolute(binary) || !path.isAbsolute(home) || !home.startsWith('/home/ubuntu/vgaps-')) throw Error('Use an absolute Gitea binary and a /home/ubuntu/vgaps-* scratch home')
  rmSync(home, { recursive: true, force: true })
  mkdirSync(path.join(home, 'work'), { recursive: true })
  execFileSync('chown', ['-R', 'ubuntu:ubuntu', home])
  const gitea = path.join(home, 'gitea')
  copyFileSync(binary, gitea); chmodSync(gitea, 0o755); execFileSync('chown', ['ubuntu:ubuntu', gitea])
  const port = await freePort()
  const config = path.join(home, 'app.ini')
  const secret = randomBytes(32).toString('hex')
  writeFileSync(config, `APP_NAME = Drill Gitea
RUN_USER = ubuntu
RUN_MODE = prod
WORK_PATH = ${home}/work

[database]
DB_TYPE = sqlite3
PATH = ${home}/work/gitea.db

[server]
HTTP_ADDR = 127.0.0.1
HTTP_PORT = ${port}
ROOT_URL = http://127.0.0.1:${port}/
DISABLE_SSH = true
OFFLINE_MODE = true
LFS_START_SERVER = false

[repository]
ROOT = ${home}/work/repos

[security]
INSTALL_LOCK = true
SECRET_KEY = ${secret}

[service]
DISABLE_REGISTRATION = true
REQUIRE_SIGNIN_VIEW = false

[log]
MODE = console
LEVEL = warn
logger.access.MODE = console
`, { mode: 0o600 })
  execFileSync('chown', ['ubuntu:ubuntu', config])
  const passwordFile = path.join(home, 'password')
  writeFileSync(passwordFile, password, { mode: 0o600 }); execFileSync('chown', ['ubuntu:ubuntu', passwordFile])
  run(gitea, ['--config', config, 'migrate'], { home, cwd: home, stdio: ['ignore', 'ignore', 'ignore'] })
  // `admin user create` takes the password as an argument: pass it only to this short-lived child.
  run(gitea, ['--config', config, 'admin', 'user', 'create', '--username', user, '--password', password, '--email', `${user}@example.invalid`, '--must-change-password=false'], { home, cwd: home, stdio: ['ignore', 'ignore', 'ignore'] })
  rmSync(passwordFile)
  let log = ''
  const child = spawn('/usr/bin/setpriv', ['--reuid=ubuntu', '--regid=ubuntu', '--clear-groups', gitea, '--config', config, 'web'], { cwd: home, env: { PATH: '/usr/bin:/bin', HOME: home }, stdio: ['ignore', 'pipe', 'pipe'] })
  for (const output of [child.stdout, child.stderr]) output.on('data', chunk => { log = (log + chunk).slice(-200_000) })
  const origin = `http://127.0.0.1:${port}`
  for (let attempt = 0; attempt < 120; attempt++) {
    if (child.exitCode !== null) throw Error('Gitea exited: ' + log.slice(-500))
    try { if ((await fetch(origin + '/api/healthz')).ok) break } catch {}
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  return {
    origin, pid: child.pid, home,
    accessLog: () => log.split('\n').filter(line => /"(GET|POST) \/(user\/(login|two_factor|logout|settings\/security)|user\/settings\/security\/two_factor)/.test(line)),
    twoFactorEnabled: async () => (await fetch(origin + '/api/v1/user', { headers: { authorization: 'Basic ' + Buffer.from(`${user}:${password}`).toString('base64') } })).status === 401,
    async stop() {
      if (child.exitCode === null && Number.isInteger(child.pid) && child.pid > 1) { child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)) }
      rmSync(home, { recursive: true, force: true })
    },
  }
}
