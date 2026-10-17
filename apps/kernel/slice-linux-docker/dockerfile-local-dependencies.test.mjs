import assert from "node:assert/strict"
import { readdir, readFile } from "node:fs/promises"
import test from "node:test"
import { join, relative, resolve } from "node:path"
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

test("slice images copy and fingerprint every repository path the kernel embeds at compile time", async () => {
  const kernelSource = fileURLToPath(new URL("../src", import.meta.url))
  const repository = fileURLToPath(new URL("../../..", import.meta.url))
  const [dockerfile, runtimeSourceRoots] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(runtimeSourceRootsPath, "utf8"),
  ])
  const roots = runtimeSourceRoots.split("\n").filter(Boolean)
  const embedded = new Set()
  const walk = async (directory) => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) await walk(path)
      else if (entry.name.endsWith(".rs") && !/(^|[\\/])tests?(\.rs|[\\/])/.test(relative(kernelSource, path))) {
        // Inline `#[cfg(test)] mod` code is not part of the runtime image build.
        const runtime = (await readFile(path, "utf8")).split(/^#\[cfg\(test\)\]\s*\nmod \w+ \{/m)[0]
        for (const match of runtime.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
          embedded.add(relative(repository, resolve(path, "..", match[1])))
        }
      }
    }
  }
  await walk(kernelSource)
  assert.ok(embedded.size > 0, "fixture should discover embedded files")
  for (const file of embedded) {
    const root = roots.find((candidate) => file === candidate || file.startsWith(`${candidate}/`))
    assert.ok(root, `runtime source roots omit ${file}`)
    assert.match(dockerfile, new RegExp(`COPY ${root.replaceAll("/", "\\/")}\\s+${root.replaceAll("/", "\\/")}`), `Dockerfile does not copy ${root} into the Rust build stage`)
  }
})
