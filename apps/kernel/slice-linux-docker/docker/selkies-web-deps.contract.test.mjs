import assert from "node:assert/strict"
import fs from "node:fs"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

export const selkiesWebDepsContractPath = fileURLToPath(import.meta.url)
const dockerDir = path.dirname(selkiesWebDepsContractPath)
const dockerfile = fs.readFileSync(path.join(dockerDir, "Dockerfile"), "utf8")

const lockFiles = [
  "selkies-web-core.package-lock.json",
  "selkies-dashboard.package-lock.json",
]

const exactVersion = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/
const integritySha512 = /^sha512-[A-Za-z0-9+/]+={0,2}$/

function readLock(filename) {
  return JSON.parse(fs.readFileSync(path.join(dockerDir, filename), "utf8"))
}

test("Selkies web lockfiles pin every resolved package and direct version", () => {
  for (const filename of lockFiles) {
    const lock = readLock(filename)
    assert.equal(lock.lockfileVersion, 3, `${filename} must use npm lockfile v3`)
    const root = lock.packages?.[""]
    assert.ok(root, `${filename} is missing its root package`)

    for (const dependencies of [root.dependencies ?? {}, root.devDependencies ?? {}]) {
      for (const [name, requested] of Object.entries(dependencies)) {
        const resolved = lock.packages[`node_modules/${name}`]
        assert.match(requested, exactVersion, `${filename}: floating direct range for ${name}`)
        assert.ok(resolved, `${filename}: missing resolved package for ${name}`)
        assert.equal(requested, resolved.version, `${filename}: direct version mismatch for ${name}`)
      }
    }

    for (const [packagePath, metadata] of Object.entries(lock.packages)) {
      if (packagePath === "") continue
      assert.match(packagePath, /^node_modules\//)
      assert.match(metadata.version ?? "", exactVersion, `${filename}: non-exact ${packagePath} version`)
      assert.match(metadata.resolved ?? "", /^https:\/\/registry\.npmjs\.org\/.+\.tgz$/)
      assert.match(metadata.integrity ?? "", integritySha512, `${filename}: missing integrity for ${packagePath}`)
    }
  }
})

test("Docker builds both web clients with the reviewed lockfiles and npm ci", () => {
  for (const filename of lockFiles) {
    assert.match(dockerfile, new RegExp(`COPY apps/kernel/slice-linux-docker/docker/${filename} /tmp/${filename}`))
  }

  const webBuild = dockerfile.slice(
    dockerfile.indexOf("RUN cd source/addons/selkies-web-core \\"),
    dockerfile.indexOf("RUN python3 -m venv /tmp/chariox-selkies-build-venv"),
  )
  assert.notEqual(webBuild.indexOf("RUN cd source/addons/selkies-web-core"), -1)
  assert.equal((webBuild.match(/npm ci --no-audit --no-fund/g) ?? []).length, 2)
  assert.doesNotMatch(webBuild, /\bnpm install\b/)
  assert.match(webBuild, /cp \/tmp\/selkies-web-core\.package-lock\.json package-lock\.json/)
  assert.match(webBuild, /cp \/tmp\/selkies-dashboard\.package-lock\.json package-lock\.json/)
})

test("platform wheel filenames remain PEP 427 names through download and hash checks", () => {
  for (const architecture of ["x86_64", "aarch64"]) {
    for (const project of ["pixelflux", "pcmflux"]) {
      const filename = `${project}-2.1.0-cp311-cp311-manylinux_2_28_${architecture}.whl`
      assert.match(dockerfile, new RegExp(`${project}_filename='${filename.replaceAll(".", "\\.")}'`))
    }
  }

  assert.match(dockerfile, /-o "\/out\/runtime-wheels\/\$pixelflux_filename"/)
  assert.match(dockerfile, /-o "\/out\/runtime-wheels\/\$pcmflux_filename"/)
  assert.doesNotMatch(dockerfile, /runtime-wheels\/(?:pixel|pcm)flux\.whl/)
})
