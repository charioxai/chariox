import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
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

// MP-08/MP-11 (r36b): agent keyboard input requires a proved AT-SPI focus.
// xterm exposes none, so the Openbox terminal must be an accessible GTK one,
// and the Room desktop carries an installed accessible editor.
test("slice image gives agents an accessible terminal and installed editor", async () => {
  const dockerfile = await readFile(dockerfilePath, "utf8")

  assert.match(dockerfile, /\n    mousepad \\\n/)
  assert.match(dockerfile, /\n    xfce4-terminal \\\n/)
  assert.match(dockerfile, /update-alternatives --set x-terminal-emulator \/usr\/bin\/xfce4-terminal\.wrapper/)
})

test("slice image renders CJK text and emoji in the Room browser", async () => {
  const dockerfile = await readFile(dockerfilePath, "utf8")

  assert.match(dockerfile, /\n    fonts-noto-cjk \\\n/)
  assert.match(dockerfile, /\n    fonts-noto-color-emoji \\\n/)
})

// MP-08/MP-10/MP-11: saved images need the same proved keyboard focus.
test("restored images upgrade to the accessible terminal and editor", async () => {
  const provisioner = await readFile(provisionerPath, "utf8")
  const refresh = provisioner.slice(provisioner.indexOf("refresh_slice_support_files()"), provisioner.indexOf("copy_required_slice_overlay", provisioner.indexOf("refresh_slice_support_files()")))
  assert.match(refresh, /command -v xfce4-terminal/)
  assert.match(refresh, /command -v mousepad/)
  assert.match(refresh, /install[^\n]*xfce4-terminal[^\n]*mousepad/)
  assert.match(refresh, /update-alternatives --set x-terminal-emulator \/usr\/bin\/xfce4-terminal\.wrapper/)
})
