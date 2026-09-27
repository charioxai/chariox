import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

export const selkiesWebDepsContractPath = fileURLToPath(import.meta.url)

const dockerfilePath = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
const buildScriptPath = fileURLToPath(new URL("./selkies-build/build.sh", import.meta.url))
const selkiesLockPath = fileURLToPath(new URL("./selkies.lock.json", import.meta.url))
const dashboardLockPath = fileURLToPath(new URL("./selkies-build/selkies-dashboard.package-lock.json", import.meta.url))
const webCoreLockPath = fileURLToPath(new URL("./selkies-build/selkies-web-core.package-lock.json", import.meta.url))

test("Selkies web dependencies stay pinned in the current build layout", async () => {
  const [dockerfile, buildScript, selkiesLockSource, dashboardLockSource, webCoreLockSource] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(buildScriptPath, "utf8"),
    readFile(selkiesLockPath, "utf8"),
    readFile(dashboardLockPath, "utf8"),
    readFile(webCoreLockPath, "utf8"),
  ])
  const selkiesLock = JSON.parse(selkiesLockSource)
  const dashboardLock = JSON.parse(dashboardLockSource)
  const webCoreLock = JSON.parse(webCoreLockSource)

  assert.match(dockerfile, /COPY apps\/kernel\/slice-linux-docker\/selkies-build\/ \.\/locks\//)
  assert.match(dockerfile, /RUN bash \/build\/locks\/build\.sh "\$TARGETARCH"/)
  assert.match(buildScript, /cp \/build\/locks\/selkies-web-core\.package-lock\.json/)
  assert.match(buildScript, /cp \/build\/locks\/selkies-dashboard\.package-lock\.json/)
  assert.equal((buildScript.match(/npm ci --no-audit --no-fund/g) ?? []).length, 2)
  assert.doesNotMatch(buildScript, /npm install(?:\s|$)/m)

  for (const [label, lock] of [["dashboard", dashboardLock], ["web core", webCoreLock]]) {
    assert.equal(lock.lockfileVersion, 3, `${label} lockfile must use npm lockfile v3`)
    assert.ok(lock.packages?.[""], `${label} lockfile must retain its root package metadata`)
    assert.ok(Object.keys(lock.packages).some((name) => name.startsWith("node_modules/")), `${label} lockfile must resolve dependencies`)
  }

  const selkiesRevision = selkiesLock.selkies?.revision
  assert.match(selkiesRevision ?? "", /^[a-f0-9]{40}$/)
  assert.match(dockerfile, new RegExp(`ARG CHARIOX_SELKIES_REVISION=${selkiesRevision}`))
  assert.match(dockerfile, /LABEL io\.chariox\.selkies-license="MPL-2\.0"/)
  assert.match(dockerfile, /LABEL io\.chariox\.selkies-source="https:\/\/github\.com\/selkies-project\/selkies\/commit\/\$\{CHARIOX_SELKIES_REVISION\}"/)
})
