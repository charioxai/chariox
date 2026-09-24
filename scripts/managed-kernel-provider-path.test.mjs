import assert from "node:assert/strict"
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

const workerServiceUrl = new URL(
  "../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service",
  import.meta.url,
)
const trustedSystemPath = "/usr/local/bin:/usr/bin:/bin"

function serviceEnvironment(unit, name) {
  const prefix = `Environment=${name}=`
  const line = unit.split("\n").find((candidate) => candidate.startsWith(prefix))
  assert.ok(line, `missing ${name} from disposable worker service`)
  return line.slice(prefix.length)
}

function resolveFixture(executable, home, path) {
  return spawnSync(executable, [], {
    encoding: "utf8",
    env: { HOME: home, PATH: path },
  })
}

test("Path-1 resolves ordinary user-local provider tools without weakening bootstrap resolution", async () => {
  const unit = await readFile(workerServiceUrl, "utf8")
  const serviceHome = serviceEnvironment(unit, "HOME")
  const servicePath = serviceEnvironment(unit, "PATH")
  const expectedServicePath = `${serviceHome}/.local/bin:${trustedSystemPath}`

  assert.equal(servicePath, expectedServicePath)
  assert.match(unit, /^ExecStart=\/usr\/local\/bin\/chariox-managed-bootstrap --disposable-worker$/m)
  assert.match(unit, /^Environment=CHARIOX_MANAGED_KERNEL_BINARY=\/usr\/local\/bin\/chariox-kernel$/m)
  assert.doesNotMatch(unit, /EnvironmentFile=|\.profile|\.bashrc|\/etc\/profile/)

  const fixtureHome = await mkdtemp(join(tmpdir(), "chariox-provider-path-"))
  try {
    const localBin = join(fixtureHome, ".local", "bin")
    const executable = "chariox-provider-path-probe"
    await mkdir(localBin, { recursive: true })
    await writeFile(join(localBin, executable), "#!/bin/sh\nprintf 'ordinary-provider-path\\n'\n")
    await chmod(join(localBin, executable), 0o700)

    const ordinaryPath = `${localBin}:${trustedSystemPath}`
    const path1Path = servicePath
      .split(":")
      .map((entry) => entry === `${serviceHome}/.local/bin` ? localBin : entry)
      .join(":")
    const ordinary = resolveFixture(executable, fixtureHome, ordinaryPath)
    const path1 = resolveFixture(executable, fixtureHome, path1Path)

    assert.equal(ordinary.status, 0, ordinary.stderr)
    assert.equal(ordinary.stdout, "ordinary-provider-path\n")
    assert.equal(path1.status, ordinary.status, path1.error?.message ?? path1.stderr)
    assert.equal(path1.stdout, ordinary.stdout)
  } finally {
    await rm(fixtureHome, { recursive: true, force: true })
  }
})
