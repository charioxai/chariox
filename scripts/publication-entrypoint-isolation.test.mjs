import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { chmod, copyFile, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { join, resolve } from "node:path"
import { tmpdir } from "node:os"
import { promisify } from "node:util"
import { test } from "node:test"

const execFileAsync = promisify(execFile)
const repositoryRoot = resolve(import.meta.dirname, "..")
const baseImage = "node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436"
const dockerSkip = await cachedDockerImageSkipReason(baseImage)
const CALLER_CLAIMS_SECRET = "caller-claims-bootstrap-secret-0123456789abcdef"

test("publication entrypoint isolates actions while the gateway authenticates to the kernel", {
  skip: dockerSkip ?? false,
  timeout: 45_000,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-isolation-"))
  const publication = join(root, "publication")
  const profile = join(root, "profile")
  const fakeKernel = join(root, "chariox-kernel")
  const fakeGateway = join(root, "gateway.js")
  const wrapper = join(root, "run.sh")
  const auditBootstrap = join(root, "audit-url")
  const callerClaimsBootstrap = join(root, "caller-claims.json")
  await copyFile(join(repositoryRoot, "docker/publication/entrypoint.sh"), join(root, "entrypoint.sh"))
  await copyFile(join(repositoryRoot, "docker/publication/wait-for-tcp.mjs"), join(root, "wait-for-tcp.mjs"))
  await mkdir(join(publication, "app"), { recursive: true })
  await mkdir(join(profile, "home", ".codex"), { recursive: true })
  await writeFile(join(profile, "home", ".codex", "credential-sentinel"), "credential-sentinel")
  await chmod(join(profile, "home", ".codex", "credential-sentinel"), 0o600)
  await writeFile(join(publication, "publication.json"), "{}\n")
  await writeFile(join(publication, "app", "actions.mjs"), actionProbeSource())
  await writeFile(fakeKernel, kernelProbeSource())
  await writeFile(fakeGateway, gatewayProbeSource())
  await writeFile(join(root, "provider-cli"), '#!/bin/sh\nprintf "trusted-%s\\n" "${0##*/}"\n', { mode: 0o755 })
  await mkdir(join(root, "untrusted-bin"))
  await writeFile(join(root, "untrusted-bin", "codex"), '#!/bin/sh\necho untrusted-codex\n', { mode: 0o755 })
  await writeFile(auditBootstrap, "http://audit-bridge.invalid/capability-sentinel")
  await chmod(auditBootstrap, 0o600)
  await writeFile(callerClaimsBootstrap, JSON.stringify({
    schema_version: 1,
    deployment_id: "deployment-bootstrap",
    environment_id: "environment-bootstrap",
    secret: CALLER_CLAIMS_SECRET,
  }))
  await chmod(callerClaimsBootstrap, 0o600)
  await writeFile(wrapper, `#!/usr/bin/env bash
set -euo pipefail
useradd --create-home --uid 1001 --shell /bin/bash chariox
useradd --create-home --uid 1002 --shell /usr/sbin/nologin chariox-action
useradd --create-home --uid 1003 --shell /usr/sbin/nologin chariox-gateway
mkdir -p /workspace/private /run/chariox-publication-capabilities/{.staging,kernel,gateway}
printf 'workspace-sentinel' > /workspace/private/kernel-work
chown -R chariox:chariox /workspace
chmod -R a+rwX /workspace
chown root:root /run/chariox-publication-capabilities /run/chariox-publication-capabilities/.staging
chown root:chariox /run/chariox-publication-capabilities/kernel
chown root:chariox-gateway /run/chariox-publication-capabilities/gateway
chmod 711 /run/chariox-publication-capabilities
chmod 700 /run/chariox-publication-capabilities/.staging
chmod 1730 /run/chariox-publication-capabilities/kernel /run/chariox-publication-capabilities/gateway
printf 'capability-victim-preserved' > /tmp/capability-victim
/usr/sbin/runuser -u chariox -- ln -s /tmp/capability-victim /run/chariox-publication-capabilities/kernel/kernel-local-auth
/usr/sbin/runuser -u chariox-gateway -- ln -s /tmp/capability-victim /run/chariox-publication-capabilities/gateway/kernel-local-auth
/usr/sbin/runuser -u chariox-gateway -- ln -s /tmp/capability-victim /run/chariox-publication-capabilities/gateway/publication-audit-url
/usr/sbin/runuser -u chariox-gateway -- ln -s /tmp/capability-victim /run/chariox-publication-capabilities/gateway/publication-caller-claims.json
cp /test/entrypoint.sh /usr/local/bin/chariox-publication-container
cp /test/wait-for-tcp.mjs /usr/local/bin/chariox-wait-for-tcp.mjs
cp /test/chariox-kernel /usr/local/bin/chariox-kernel
mkdir -p /opt/chariox/apps/server/dist
cp /test/gateway.js /opt/chariox/apps/server/dist/index.js
mkdir -p /opt/chariox-toolchain/node_modules/.bin
for provider in codex opencode claude; do
  cp /test/provider-cli "/opt/chariox-toolchain/node_modules/.bin/$provider"
done
chown -R root:root /opt/chariox-toolchain
chmod -R go-w /opt/chariox-toolchain
chmod 755 /usr/local/bin/chariox-publication-container /usr/local/bin/chariox-kernel
cp -a /test/publication /publication
cp -a /test/profile /home/chariox/.provider-credentials
exec /usr/local/bin/chariox-publication-container standalone
`)
  await Promise.all([chmod(fakeKernel, 0o755), chmod(wrapper, 0o755)])

  try {
    const failure = await execFileAsync("docker", [
      "run",
      "--rm",
      "--security-opt",
      "no-new-privileges",
      "--cap-drop",
      "ALL",
      "--cap-add",
      "CHOWN",
      "--cap-add",
      "SETUID",
      "--cap-add",
      "SETGID",
      "--cap-add",
      "DAC_OVERRIDE",
      "--cap-add",
      "FOWNER",
      "--cap-add",
      "KILL",
      "--entrypoint",
      "/test/run.sh",
      "-e",
      "PATH=/test/untrusted-bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin",
      "-e",
      "HOME=/home/chariox",
      "-e",
      "CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox",
      "-e",
      "CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox",
      "-e",
      "CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime",
      "-e",
      "CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions",
      "-e",
      "CHARIOX_PROVIDER_CREDENTIALS_DIR=/home/chariox/.provider-credentials",
      "-e",
      "CHARIOX_PUBLICATION_PACKAGE=/publication",
      "-e",
      "CHARIOX_PUBLICATION_RUNNER_KEY=runner-env-sentinel",
      "-e",
      "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY=cloud-runner-env-sentinel",
      "-e",
      "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE=/test/audit-url",
      "-e",
      "CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE=/test/caller-claims.json",
      "-e",
      "OPENAI_API_KEY=provider-env-sentinel",
      "-e",
      "CHARIOX_RELAY_URL=wss://relay.example.test",
      "-e",
      "CHARIOX_RELAY_TOKEN=relay-token-sentinel",
      "-e",
      "CHARIOX_CLOUD_RELAY_CONFIG_JSON={\"cloud_relay\":{\"relayUrl\":\"wss://cloud-relay.example.test\"}}",
      "-e",
      "CHARIOX_RELAY_HEARTBEAT_MS=12345",
      "-e",
      "CHARIOX_KERNEL_QUEUE_CAPACITY=321",
      "-e",
      "NODE_EXTRA_CA_CERTS=/etc/ssl/certs/ca-certificates.crt",
      "-e",
      "HTTPS_PROXY=http://proxy.example.test:8080",
      "-e",
      "NO_PROXY=127.0.0.1,localhost",
      "-e",
      "CHARIOX_PUBLICATION_CLOUD_API_URL=https://self-host.example.test",
      "-e",
      "CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID=self-host-account",
      "-e",
      "CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN=self-host-session-sentinel",
      "-e",
      "CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID=self-host-deployment",
      "-v",
      `${root}:/test:ro`,
      baseImage,
    ], { timeout: 40_000, maxBuffer: 1024 * 1024 }).then(
      () => null,
      (error) => error,
    )

    assert.ok(failure)
    assert.equal(failure.code, 70, String(failure.stderr))
    const stdout = String(failure.stdout)
    assert.match(stdout, /provider readiness read credential-sentinel without inheriting kernel auth/)
    assert.match(stdout, /action uid 1002 denied credential, workspace, and unauthenticated kernel access/)
    assert.match(stdout, /gateway uid 1003 authenticated with isolated audit and caller claims capabilities/)
    assert.match(stdout, /kernel resolves immutable provider CLIs/)
    assert.match(stdout, /gateway resolves immutable provider CLIs/)
    assert.match(stdout, /capability symlinks replaced without modifying their target/)
    assert.match(String(failure.stderr), /publication standalone child exited: gateway \(status 0\)/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication entrypoint exits and cleans up when the kernel or action server drops", {
  skip: dockerSkip ?? false,
  timeout: 45_000,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-supervision-"))
  const publication = join(root, "publication")
  const fakeKernel = join(root, "chariox-kernel")
  const fakeGateway = join(root, "gateway.js")
  const wrapper = join(root, "run.sh")
  await copyFile(join(repositoryRoot, "docker/publication/entrypoint.sh"), join(root, "entrypoint.sh"))
  await copyFile(join(repositoryRoot, "docker/publication/wait-for-tcp.mjs"), join(root, "wait-for-tcp.mjs"))
  await mkdir(join(publication, "app"), { recursive: true })
  await writeFile(join(publication, "publication.json"), "{}\n")
  await writeFile(join(publication, "app", "actions.mjs"), supervisedActionSource())
  await writeFile(fakeKernel, supervisedKernelSource())
  await writeFile(fakeGateway, supervisedGatewaySource())
  await writeFile(wrapper, `#!/usr/bin/env bash
set -euo pipefail
useradd --create-home --uid 1001 --shell /bin/bash chariox
useradd --create-home --uid 1002 --shell /usr/sbin/nologin chariox-action
useradd --create-home --uid 1003 --shell /usr/sbin/nologin chariox-gateway
touch "/tmp/fail-\${FAIL_CHILD:?}"
cp /test/entrypoint.sh /usr/local/bin/chariox-publication-container
cp /test/wait-for-tcp.mjs /usr/local/bin/chariox-wait-for-tcp.mjs
cp /test/chariox-kernel /usr/local/bin/chariox-kernel
mkdir -p /opt/chariox/apps/server/dist
cp /test/gateway.js /opt/chariox/apps/server/dist/index.js
chmod 755 /usr/local/bin/chariox-publication-container /usr/local/bin/chariox-kernel
cp -a /test/publication /publication
exec /usr/local/bin/chariox-publication-container standalone
`)
  await Promise.all([chmod(fakeKernel, 0o755), chmod(wrapper, 0o755)])

  const scenarios = [
    {
      failure: "kernel-startup",
      label: "kernel",
      status: 25,
      siblingMessages: ["supervision action terminated"],
    },
    {
      failure: "action-startup",
      label: "action server",
      status: 26,
      siblingMessages: ["supervision kernel terminated"],
    },
    {
      failure: "kernel",
      label: "kernel",
      status: 23,
      siblingMessages: ["supervision action terminated", "supervision gateway terminated"],
    },
    {
      failure: "action",
      label: "action server",
      status: 24,
      siblingMessages: ["supervision kernel terminated", "supervision gateway terminated"],
    },
  ]
  try {
    for (const scenario of scenarios) {
      const failure = await execFileAsync("docker", [
        "run",
        "--rm",
        "--security-opt",
        "no-new-privileges",
        "--cap-drop",
        "ALL",
        "--cap-add",
        "CHOWN",
        "--cap-add",
        "SETUID",
        "--cap-add",
        "SETGID",
        "--cap-add",
        "DAC_OVERRIDE",
        "--cap-add",
        "FOWNER",
        "--cap-add",
        "KILL",
        "--entrypoint",
        "/test/run.sh",
        "-e",
        `FAIL_CHILD=${scenario.failure}`,
        "-e",
        "HOME=/home/chariox",
        "-e",
        "CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox",
        "-e",
        "CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox",
        "-e",
        "CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime",
        "-e",
        "CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions",
        "-e",
        "CHARIOX_PUBLICATION_PACKAGE=/publication",
        "-v",
        `${root}:/test:ro`,
        baseImage,
      ], { timeout: 20_000, maxBuffer: 512 * 1024 }).then(
        () => null,
        (error) => error,
      )
      assert.ok(failure)
      assert.equal(failure.code, scenario.status, String(failure.stderr))
      assert.match(
        String(failure.stderr),
        new RegExp(`publication standalone child exited: ${scenario.label} \\(status ${scenario.status}\\)`),
      )
      for (const message of scenario.siblingMessages) {
        assert.match(
          String(failure.stdout),
          new RegExp(message),
          `${scenario.failure} failure did not clean up sibling: ${String(failure.stderr)}`,
        )
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication split kernel and gateway modes fail closed without local auth files", {
  skip: dockerSkip ?? false,
  timeout: 30_000,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-split-auth-"))
  const wrapper = join(root, "run.sh")
  await copyFile(join(repositoryRoot, "docker/publication/entrypoint.sh"), join(root, "entrypoint.sh"))
  await writeFile(wrapper, `#!/usr/bin/env bash
set -euo pipefail
useradd --create-home --uid 1001 --shell /bin/bash chariox
useradd --create-home --uid 1002 --shell /usr/sbin/nologin chariox-action
useradd --create-home --uid 1003 --shell /usr/sbin/nologin chariox-gateway
mkdir -p /publication
cp /test/entrypoint.sh /usr/local/bin/chariox-publication-container
chmod 755 /usr/local/bin/chariox-publication-container
set +e
kernel_output="$(/usr/local/bin/chariox-publication-container kernel 2>&1)"
kernel_status=$?
gateway_output="$(/usr/local/bin/chariox-publication-container gateway 2>&1)"
gateway_status=$?
set -e
test "$kernel_status" -eq 70
test "$gateway_status" -eq 70
grep -q 'publication kernel local auth token file is required' <<<"$kernel_output"
grep -q 'publication kernel local auth token file is required' <<<"$gateway_output"
printf 'split kernel and gateway rejected missing local auth files\n'
`)
  await chmod(wrapper, 0o755)

  try {
    const { stdout } = await execFileAsync("docker", [
      "run",
      "--rm",
      "--entrypoint",
      "/test/run.sh",
      "-e",
      "HOME=/home/chariox",
      "-e",
      "CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox",
      "-e",
      "CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox",
      "-e",
      "CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime",
      "-e",
      "CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions",
      "-v",
      `${root}:/test:ro`,
      baseImage,
    ], { timeout: 25_000, maxBuffer: 256 * 1024 })
    assert.match(stdout, /split kernel and gateway rejected missing local auth files/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

async function cachedDockerImageSkipReason(image) {
  try {
    await execFileAsync("docker", ["info"], { timeout: 5_000 })
    await execFileAsync("docker", ["image", "inspect", image], { timeout: 5_000 })
    return null
  } catch {
    return `requires Docker and cached ${image}`
  }
}

function kernelProbeSource() {
  return `#!/usr/bin/env node
const assert = require("node:assert/strict")
const fs = require("node:fs")
const net = require("node:net")
const { spawnSync } = require("node:child_process")
assert.equal(process.getuid(), 1001)
${providerToolchainProbeSource("kernel")}
assertUnprivilegedCapabilities()
assertPrivateTempDirectory("/home/chariox/.tmp", 1001)
assert.equal(process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, undefined)
const tokenFile = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
assert.ok(tokenFile)
const tokenStat = fs.statSync(tokenFile)
assert.equal(tokenStat.uid, 1001)
assert.equal(tokenStat.mode & 0o777, 0o600)
const token = fs.readFileSync(tokenFile, "utf8")
assert.ok(token && token.length >= 64)
assert.equal(fs.lstatSync(tokenFile).isSymbolicLink(), false)
assert.equal(fs.readFileSync("/tmp/capability-victim", "utf8"), "capability-victim-preserved")
fs.unlinkSync(tokenFile)
delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
assert.equal(process.env.CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL, undefined)
assert.equal(process.env.CHARIOX_PUBLICATION_RUNNER_KEY, undefined)
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY, undefined)
assert.equal(process.env.CHARIOX_RELAY_URL, "wss://relay.example.test")
assert.equal(process.env.CHARIOX_RELAY_TOKEN, "relay-token-sentinel")
assert.match(process.env.CHARIOX_CLOUD_RELAY_CONFIG_JSON, /cloud-relay\.example\.test/)
assert.equal(process.env.CHARIOX_RELAY_HEARTBEAT_MS, "12345")
assert.equal(process.env.CHARIOX_KERNEL_QUEUE_CAPACITY, "321")
assert.equal(process.env.NODE_EXTRA_CA_CERTS, "/etc/ssl/certs/ca-certificates.crt")
assert.equal(process.env.HTTPS_PROXY, "http://proxy.example.test:8080")
assert.equal(process.env.NO_PROXY, "127.0.0.1,localhost")
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN, undefined)
assert.equal(fs.readFileSync("/home/chariox/.codex/credential-sentinel", "utf8"), "credential-sentinel")
const workspaceStat = fs.statSync("/workspace")
assert.equal(workspaceStat.uid, 1001)
assert.equal(workspaceStat.mode & 0o777, 0o700)
const workspaceFileStat = fs.statSync("/workspace/private/kernel-work")
assert.equal(workspaceFileStat.uid, 1001)
assert.equal(workspaceFileStat.mode & 0o077, 0)
assert.equal(fs.readFileSync("/workspace/private/kernel-work", "utf8"), "workspace-sentinel")
assert.throws(
  () => fs.writeFileSync("/opt/chariox/apps/server/dist/index.js", "kernel-overwrite"),
  /EACCES|EPERM/,
)
const child = spawnSync(process.execPath, ["-e", "const fs=require('node:fs'); const parent=fs.readFileSync('/proc/' + process.ppid + '/environ', 'utf8'); if (Object.keys(process.env).some((name) => name.includes('AUTH_TOKEN') || name.includes('AUDIT')) || parent.includes('" + token + "') || parent.includes('capability-sentinel')) process.exit(2)"], {
  env: { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR },
})
assert.equal(child.status, 0)
console.log("provider readiness read credential-sentinel without inheriting kernel auth")
net.createServer((socket) => {
  socket.once("data", (chunk) => {
    const request = String(chunk)
    if (request.includes("Authorization: Bearer " + token + "\\r\\n")) {
      socket.end("HTTP/1.1 101 Switching Protocols\\r\\nUpgrade: websocket\\r\\nConnection: Upgrade\\r\\n\\r\\n")
    } else {
      socket.end("HTTP/1.1 401 Unauthorized\\r\\nContent-Length: 0\\r\\nConnection: close\\r\\n\\r\\n")
    }
  })
}).listen(43118, "127.0.0.1")
function assertUnprivilegedCapabilities() {
  const status = fs.readFileSync("/proc/self/status", "utf8")
  assert.match(status, /^CapPrm:\\s+0+$/m)
  assert.match(status, /^CapEff:\\s+0+$/m)
}
function assertPrivateTempDirectory(path, uid) {
  assert.equal(process.env.TMPDIR, path)
  const stat = fs.statSync(path)
  assert.equal(stat.uid, uid)
  assert.equal(stat.mode & 0o777, 0o700)
}
`
}

function actionProbeSource() {
  return `import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { readFile, readdir, stat, writeFile } from "node:fs/promises"
import net from "node:net"
assert.equal(process.getuid(), 1002)
assert.equal(process.env.PATH, "/usr/local/bin:/usr/bin:/bin")
assert.equal(process.env.HOME, "/home/chariox-action")
assert.equal(process.cwd(), "/publication/app")
assert.equal(process.env.TMPDIR, "/home/chariox-action/.tmp")
const tempStat = await stat(process.env.TMPDIR)
assert.equal(tempStat.uid, 1002)
assert.equal(tempStat.mode & 0o777, 0o700)
const processStatus = readFileSync("/proc/self/status", "utf8")
assert.match(processStatus, /^CapPrm:\\s+0+$/m)
assert.match(processStatus, /^CapEff:\\s+0+$/m)
const boundingMatch = processStatus.match(/^CapBnd:\\s+([a-f0-9]+)$/mi)
assert.ok(boundingMatch)
const bounding = BigInt("0x" + boundingMatch[1])
assert.equal((bounding & (1n << 13n)) === 0n, true)
for (const name of [
  "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
  "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
  "CHARIOX_PUBLICATION_RUNNER_KEY",
  "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE",
  "CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE",
  "OPENAI_API_KEY",
  "CHARIOX_RELAY_URL",
  "CHARIOX_RELAY_TOKEN",
  "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
  "NODE_EXTRA_CA_CERTS",
  "HTTPS_PROXY",
  "NO_PROXY",
  "CHARIOX_PUBLICATION_CLOUD_API_URL",
  "CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID",
  "CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN",
]) assert.equal(process.env[name], undefined)
assert.equal(Object.values(process.env).some((value) => value.includes("capability-sentinel")), false)
assert.equal(Object.values(process.env).some((value) => value.includes("${CALLER_CLAIMS_SECRET}")), false)
for (const path of [
  "/home/chariox/.codex/credential-sentinel",
  "/home/chariox/.provider-credentials/home/.codex/credential-sentinel",
]) await assert.rejects(readFile(path, "utf8"), /EACCES|EPERM/)
await assert.rejects(readFile("/workspace/private/kernel-work", "utf8"), /EACCES|EPERM/)
await assert.rejects(writeFile("/workspace/private/kernel-work", "action-overwrite"), /EACCES|EPERM/)
await assert.rejects(writeFile("/workspace/action-created", "action-create"), /EACCES|EPERM/)
await assert.rejects(readdir("/workspace"), /EACCES|EPERM/)
for (const path of ["/home/chariox/.tmp", "/home/chariox-gateway/.tmp"]) {
  await assert.rejects(readdir(path), /EACCES|EPERM/)
}
await assert.rejects(readFile("/proc/1/environ"), /EACCES|EPERM/)
await assertKernelDeniesUnauthenticated()
await assertGatewayEnvironmentHidden()
console.log("action uid 1002 denied credential, workspace, and unauthenticated kernel access")
await writeFile("/tmp/chariox-action-isolation-complete", "complete")
setInterval(() => {}, 1000)
async function assertKernelDeniesUnauthenticated() {
  const deadline = Date.now() + 3000
  for (;;) {
    try {
      const response = await new Promise((resolve, reject) => {
        const socket = net.createConnection({ host: "127.0.0.1", port: 43118 })
        const timer = setTimeout(() => {
          socket.destroy()
          reject(new Error("kernel denial timed out"))
        }, 3000)
        socket.once("connect", () => socket.write("GET / HTTP/1.1\\r\\nHost: 127.0.0.1\\r\\nUpgrade: websocket\\r\\nConnection: Upgrade\\r\\n\\r\\n"))
        socket.once("data", (chunk) => {
          clearTimeout(timer)
          socket.destroy()
          resolve(String(chunk))
        })
        socket.once("error", (error) => {
          clearTimeout(timer)
          reject(error)
        })
      })
      assert.match(response, /^HTTP\\/1\\.1 401 /)
      return
    } catch (error) {
      if (error?.code !== "ECONNREFUSED" || Date.now() >= deadline) throw error
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
  }
}
async function assertGatewayEnvironmentHidden() {
  const deadline = Date.now() + 3000
  for (;;) {
    for (const entry of await readdir("/proc")) {
      if (!/^\\d+$/.test(entry)) continue
      try {
        const status = await readFile("/proc/" + entry + "/status", "utf8")
        if (!/^Uid:\\s+1003\\s/m.test(status)) continue
        await assert.rejects(readFile("/proc/" + entry + "/environ"), /EACCES|EPERM/)
        return
      } catch (error) {
        if (error?.code !== "ENOENT" && error?.code !== "ESRCH") throw error
      }
    }
    if (Date.now() >= deadline) throw new Error("gateway process not found")
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
}
`
}

function supervisedKernelSource() {
  return `#!/usr/bin/env node
const fs = require("node:fs")
const net = require("node:net")
const server = net.createServer()
process.on("SIGTERM", () => {
  console.log("supervision kernel terminated")
  process.exit(0)
})
if (fs.existsSync("/tmp/fail-kernel-startup")) {
  setTimeout(() => process.exit(25), 400)
} else {
  const listenDelay = fs.existsSync("/tmp/fail-action-startup") ? 2000 : 0
  setTimeout(() => server.listen(43118, "127.0.0.1", () => {
    if (fs.existsSync("/tmp/fail-kernel")) {
      setTimeout(() => process.exit(23), 1000)
    }
  }), listenDelay)
}
`
}

function supervisedActionSource() {
  return `import fs from "node:fs"
process.on("SIGTERM", () => {
  console.log("supervision action terminated")
  process.exit(0)
})
if (fs.existsSync("/tmp/fail-action")) {
  setTimeout(() => process.exit(24), 1000)
}
if (fs.existsSync("/tmp/fail-action-startup")) {
  setTimeout(() => process.exit(26), 400)
}
setInterval(() => {}, 1000)
`
}

function supervisedGatewaySource() {
  return `process.on("SIGTERM", () => {
  console.log("supervision gateway terminated")
  process.exit(0)
})
setInterval(() => {}, 1000)
`
}

function gatewayProbeSource() {
  return `const assert = require("node:assert/strict")
const fs = require("node:fs")
const net = require("node:net")
const { spawnSync } = require("node:child_process")
assert.equal(process.getuid(), 1003)
${providerToolchainProbeSource("gateway")}
assert.equal(process.env.HOME, "/home/chariox-gateway")
assert.equal(process.cwd(), "/publication")
assert.equal(process.env.TMPDIR, "/home/chariox-gateway/.tmp")
const tempStat = fs.statSync(process.env.TMPDIR)
assert.equal(tempStat.uid, 1003)
assert.equal(tempStat.mode & 0o777, 0o700)
const processStatus = fs.readFileSync("/proc/self/status", "utf8")
assert.match(processStatus, /^CapPrm:\\s+0+$/m)
assert.match(processStatus, /^CapEff:\\s+0+$/m)
for (const name of ["CHARIOX_PUBLICATION_RUNNER_KEY", "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY", "OPENAI_API_KEY"]) {
  assert.equal(process.env[name], undefined)
}
assert.equal(process.env.CHARIOX_RELAY_URL, undefined)
assert.equal(process.env.CHARIOX_RELAY_TOKEN, undefined)
assert.equal(process.env.CHARIOX_CLOUD_RELAY_CONFIG_JSON, undefined)
assert.equal(process.env.NODE_EXTRA_CA_CERTS, "/etc/ssl/certs/ca-certificates.crt")
assert.equal(process.env.HTTPS_PROXY, "http://proxy.example.test:8080")
assert.equal(process.env.NO_PROXY, "127.0.0.1,localhost")
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_API_URL, "https://self-host.example.test")
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID, "self-host-account")
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN, "self-host-session-sentinel")
assert.equal(process.env.CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID, "self-host-deployment")
assert.throws(() => fs.readFileSync("/home/chariox/.codex/credential-sentinel"), /EACCES|EPERM/)
assert.equal(process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, undefined)
assert.equal(process.env.CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL, undefined)
const tokenFile = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
const auditFile = process.env.CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE
const callerClaimsFile = process.env.CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE
assert.ok(tokenFile && auditFile && callerClaimsFile)
const token = fs.readFileSync(tokenFile, "utf8")
const auditUrl = fs.readFileSync(auditFile, "utf8")
const callerClaims = fs.readFileSync(callerClaimsFile, "utf8")
assert.ok(token && token.length >= 64)
assert.equal(auditUrl, "http://audit-bridge.invalid/capability-sentinel")
assert.deepEqual(JSON.parse(callerClaims), {
  schema_version: 1,
  deployment_id: "deployment-bootstrap",
  environment_id: "environment-bootstrap",
  secret: "${CALLER_CLAIMS_SECRET}",
})
assert.equal(fs.lstatSync(tokenFile).isSymbolicLink(), false)
assert.equal(fs.lstatSync(auditFile).isSymbolicLink(), false)
assert.equal(fs.lstatSync(callerClaimsFile).isSymbolicLink(), false)
assert.equal(fs.statSync(callerClaimsFile).uid, 1003)
assert.equal(fs.statSync(callerClaimsFile).mode & 0o777, 0o600)
assert.equal(fs.readFileSync("/tmp/capability-victim", "utf8"), "capability-victim-preserved")
console.log("capability symlinks replaced without modifying their target")
fs.unlinkSync(tokenFile)
fs.unlinkSync(auditFile)
fs.unlinkSync(callerClaimsFile)
delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
delete process.env.CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE
delete process.env.CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE
const child = spawnSync(process.execPath, ["-e", "const fs=require('node:fs'); const parent=fs.readFileSync('/proc/' + process.ppid + '/environ', 'utf8'); if (Object.keys(process.env).some((name) => name.includes('AUTH_TOKEN') || name.includes('AUDIT') || name.includes('CALLER_CLAIMS')) || parent.includes('" + token + "') || parent.includes('capability-sentinel') || parent.includes('${CALLER_CLAIMS_SECRET}')) process.exit(2)"], {
  env: { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR },
})
assert.equal(child.status, 0)
const socket = net.createConnection({ host: "127.0.0.1", port: 43118 })
socket.once("connect", () => socket.write(
  "GET / HTTP/1.1\\r\\nHost: 127.0.0.1\\r\\nUpgrade: websocket\\r\\nConnection: Upgrade\\r\\nAuthorization: Bearer " + token + "\\r\\n\\r\\n",
))
socket.once("data", (chunk) => {
  assert.match(String(chunk), /^HTTP\\/1\\.1 101 /)
  console.log("gateway uid 1003 authenticated with isolated audit and caller claims capabilities")
  socket.destroy()
  const completion = setInterval(() => {
    if (fs.existsSync("/tmp/chariox-action-isolation-complete")) {
      clearInterval(completion)
      process.exit(0)
    }
  }, 25)
})
socket.once("error", (error) => { throw error })
setTimeout(() => { throw new Error("authenticated gateway timed out") }, 3000).unref()
`
}

function providerToolchainProbeSource(role) {
  return `for (const provider of ["codex", "opencode", "claude"]) {
  const probe = spawnSync(provider, ["--version"], { encoding: "utf8", timeout: 2000 })
  assert.equal(probe.status, 0, provider + " must resolve after publication environment isolation: " + (probe.error?.message ?? probe.stderr))
  assert.equal(probe.stdout.trim(), "trusted-" + provider)
  const binary = "/opt/chariox-toolchain/node_modules/.bin/" + provider
  assert.equal(fs.statSync(binary).uid, 0)
  assert.throws(() => fs.writeFileSync(binary, "untrusted replacement"), /EACCES|EPERM/)
}
console.log("${role} resolves immutable provider CLIs")`
}
