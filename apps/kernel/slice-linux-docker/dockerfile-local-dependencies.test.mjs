import assert from "node:assert/strict"
import { glob, readFile } from "node:fs/promises"
import { dirname, join, relative, resolve } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const kernelManifest = fileURLToPath(new URL("../Cargo.toml", import.meta.url))
const dockerfilePath = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
const provisionerPath = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const runtimeSourceRootsPath = fileURLToPath(new URL("./runtime-source-roots.txt", import.meta.url))
const selkiesLifecycleTestPath = fileURLToPath(new URL("./docker/test_slice_selkies.py", import.meta.url))

test("slice images copy and fingerprint every kernel package path dependency", async () => {
  const [manifest, dockerfile, provisioner, runtimeSourceRoots] = await Promise.all([
    readFile(kernelManifest, "utf8"),
    readFile(dockerfilePath, "utf8"),
    readFile(provisionerPath, "utf8"),
    readFile(runtimeSourceRootsPath, "utf8"),
  ])
  const roots = runtimeSourceRoots.split("\n").filter(Boolean)
  const dependencies = [...manifest.matchAll(/path\s*=\s*"\.\.\/\.\.\/packages\/([^"/]+)"/g)]
    .map((match) => match[1])
  assert.ok(dependencies.length > 0, "fixture should discover at least one local package dependency")
  assert.match(provisioner, /runtime-source-roots\.txt/)
  for (const dependency of dependencies) {
    assert.match(dockerfile, new RegExp(`packages/${dependency}\\s+(?:/opt/chariox-source/)?packages/${dependency}`))
    assert.ok(roots.includes(`packages/${dependency}`), `runtime source roots omit packages/${dependency}`)
  }
})

test("slice image build runs the discoverable Selkies lifecycle tests", async () => {
  const [dockerfile, lifecycleTest] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(selkiesLifecycleTestPath, "utf8"),
  ])

  assert.match(lifecycleTest, /class SelkiesStopTests/)
  assert.match(dockerfile, /python[^\n]*-m unittest discover\s*\\?\s*-s [^\n]* -p 'test_\*\.py'/)
})

test("slice image provides the desktop terminal command advertised by Openbox", async () => {
  const dockerfile = await readFile(dockerfilePath, "utf8")

  assert.match(dockerfile, /\n    xterm \\\n/)
  assert.match(dockerfile, /test -x \/usr\/bin\/x-terminal-emulator/)
})

test("slice image renders CJK text and emoji in the Room browser", async () => {
  const dockerfile = await readFile(dockerfilePath, "utf8")

  assert.match(dockerfile, /\n    fonts-noto-cjk \\\n/)
  assert.match(dockerfile, /\n    fonts-noto-color-emoji \\\n/)
})

test("slice images carry and fingerprint every deploy script the kernel embeds at compile time", async () => {
  const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url))
  const [dockerfile, runtimeSourceRoots] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(runtimeSourceRootsPath, "utf8"),
  ])
  const roots = runtimeSourceRoots.split("\n").filter(Boolean)
  const copiedPackages = [...dockerfile.matchAll(/^COPY\s+(?:--\S+\s+)*(\S+)\s+\S+$/gm)].map((match) => match[1])
  const embedded = new Set()
  for (const sourceRoot of ["apps/kernel/src", "apps/relay/src"]) {
    for await (const file of glob(`${sourceRoot}/**/*.rs`, { cwd: repositoryRoot })) {
      if (file.endsWith("/tests.rs") || file.includes("/tests/")) continue // test builds do not run in the image
      const source = await readFile(join(repositoryRoot, file), "utf8")
      for (const match of source.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
        const target = relative(repositoryRoot, resolve(repositoryRoot, dirname(file), match[1]))
        if (target.startsWith("deploy/") && !copiedPackages.some((copied) => target === copied || target.startsWith(`${copied}/`))) embedded.add(target)
      }
    }
  }
  for (const target of embedded) {
    assert.ok(dockerfile.includes(target), `the slice Dockerfile does not copy ${target}, which the kernel embeds with include_str!/include_bytes!`)
    assert.ok(roots.includes(target), `runtime-source-roots.txt does not fingerprint ${target}`)
  }
})
