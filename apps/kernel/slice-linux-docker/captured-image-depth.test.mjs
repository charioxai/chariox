import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import {
  MAX_CAPTURED_IMAGE_LAYERS, captureContainerImage, captureNeedsFlatten, flattenContainerImage, flattenedConfigMismatches,
  flattenedImageId, importChanges,
} from "./captured-image-depth.mjs"
import { recordCapturedImageProof, recordFlattenedImageProof, recordManagedImageProof } from "./protected-image-proof.mjs"
import { recordLegacyImageProof } from "./legacy-image-proof.mjs"
import { readProtectedLayoutReceipt } from "./protected-layout-store.mjs"

const layer = (n) => `sha256:${String(n).padStart(64, "0")}`
const image = (n) => `sha256:${String(n).padStart(64, "a")}`
// Proof stores refuse group/world-writable ancestors such as /tmp.
const privateRoot = (label) => mkdtemp(join(homedir(), `.chariox-${label}-`))

test("captures flatten exactly when the parent reaches the depth bound", () => {
  assert.equal(captureNeedsFlatten(1), false)
  assert.equal(captureNeedsFlatten(MAX_CAPTURED_IMAGE_LAYERS - 1), false)
  assert.equal(captureNeedsFlatten(MAX_CAPTURED_IMAGE_LAYERS), true)
  assert.equal(captureNeedsFlatten(124), true)
  assert.ok(MAX_CAPTURED_IMAGE_LAYERS < 125, "Docker's overlay2 store refuses images deeper than 125 layers")
  for (const invalid of [undefined, 0, -1, 1.5, "100"]) assert.throws(() => captureNeedsFlatten(invalid), /depth is unavailable/)
})

test("the kernel and the brokers share one depth bound", async () => {
  const kernel = await readFile(new URL("../src/slice/local_docker/capture_depth.rs", import.meta.url), "utf8")
  assert.equal(Number(kernel.match(/const MAX_CAPTURED_IMAGE_LAYERS: usize = (\d+);/)[1]), MAX_CAPTURED_IMAGE_LAYERS)
  const broker = await readFile(new URL("./managed-docker-broker.mjs", import.meta.url), "utf8")
  assert.match(broker, /flattenCapture = captureNeedsFlatten\(commitParent\.RootFS\?\.Layers\?\.length\)/)
  assert.match(broker, /"captured-image-depth\.mjs"\),\s*"flatten", commitSource\.Id, prepared\.args\[2\]\]/)
  // The kernel's admission and routing probes are allowed broker shapes.
  for (const shape of ['"{{.Image}}"', '"{{.SizeRootFs}}"', '"{{len .RootFS.Layers}}"']) assert.ok(broker.includes(shape), shape)
})

test("import restates the container configuration that commit would keep", () => {
  const config = {
    Env: ["PATH=/usr/local/bin:/usr/bin", "QUOTED=a \"b\" \\c $HOME"],
    User: "slice", WorkingDir: "/workspace",
    Entrypoint: ["docker-entrypoint.sh"], Cmd: ["sleep", "infinity"],
    Labels: { "io.chariox.slice.id": "slice-1", "io.chariox.runtime-source-revision": "abc$1" },
    ExposedPorts: { "6080/tcp": {} }, Volumes: null, StopSignal: "SIGRTMIN+3", Healthcheck: null, OnBuild: null,
  }
  const changes = importChanges(config)
  assert.ok(changes.length % 2 === 0 && changes.filter((_, i) => i % 2 === 0).every((flag) => flag === "--change"))
  assert.deepEqual(changes.filter((_, i) => i % 2 === 1), [
    'ENV PATH="/usr/local/bin:/usr/bin"',
    'ENV QUOTED="a \\"b\\" \\\\c \\$HOME"',
    "USER slice",
    'WORKDIR "/workspace"',
    'ENTRYPOINT ["docker-entrypoint.sh"]',
    'CMD ["sleep","infinity"]',
    'LABEL "io.chariox.slice.id"="slice-1"',
    'LABEL "io.chariox.runtime-source-revision"="abc\\$1"',
    "EXPOSE 6080/tcp",
    "STOPSIGNAL SIGRTMIN+3",
  ])
  for (const [field, value, reason] of [
    ["Healthcheck", { Test: ["CMD", "true"] }, /health check/],
    ["OnBuild", ["RUN true"], /ONBUILD/],
    ["Env", ["BAD NAME=1"], /NAME=value/],
    ["Env", ["MULTI=a\nb"], /single line/],
    ["User", "slice; rm", /plain name/],
    ["ExposedPorts", { "80/xyz": {} }, /port/],
  ]) assert.throws(() => importChanges({ ...config, [field]: value }), reason)
  assert.deepEqual(flattenedConfigMismatches(config, { ...config, Volumes: {}, OnBuild: [] }), [])
  assert.deepEqual(flattenedConfigMismatches(config, { ...config, Env: config.Env.slice(1), User: "root" }), ["Env", "User"])
})

test("the broker reads the imported image ID from the flatten helper's output", () => {
  const id = image(42)
  assert.equal(flattenedImageId(Buffer.from(`noise\n${JSON.stringify({ image: id, layers: 1 })}\n`)), id)
  for (const output of ["", "{}", '{"image":"latest"}', "not json"]) {
    assert.throws(() => flattenedImageId(Buffer.from(output)), /did not report its image/)
  }
})

test("a flattened capture restarts protected lineage at one layer and keeps the runtime proof", async () => {
  const root = await privateRoot("flatten-proof")
  try {
    const digest = `sha256:${"b".repeat(64)}`, kernelHash = "c".repeat(64)
    const env = ["PATH=/usr/bin", "CHARIOX_SLICE_ID=slice-1"]
    const base = { Id: image(1), Config: { User: "slice", Env: env }, RootFS: { Layers: Array.from({ length: 124 }, (_, i) => layer(i + 1)) } }
    recordManagedImageProof(root, digest, base, kernelHash)
    const container = { Id: "d".repeat(64), Image: base.Id, Config: { User: "slice", Env: env } }
    const flat = { Id: image(2), Parent: "", Config: { User: "slice", Env: env }, RootFS: { Layers: [layer(900)] } }
    for (const refused of [
      { ...flat, RootFS: { Layers: [layer(900), layer(901)] } },
      { ...flat, Parent: base.Id },
      { ...flat, Config: { User: "slice", Env: env.slice(1) } },
      { ...flat, Config: { User: "root", Env: env } },
    ]) assert.throws(() => recordFlattenedImageProof(root, digest, base, container, refused, refused.Id), /trusted managed runtime/)
    assert.throws(() => recordFlattenedImageProof(root, `sha256:${"e".repeat(64)}`, base, container, flat, flat.Id))
    // The tag must still name the image the helper imported.
    assert.throws(() => recordFlattenedImageProof(root, digest, base, container, flat, image(9)), /trusted managed runtime/)
    recordFlattenedImageProof(root, digest, base, container, flat, flat.Id)
    const receipt = readProtectedLayoutReceipt(root, flat.Id.slice(7))
    assert.deepEqual([receipt.parentImageId, receipt.layers, receipt.kernelHash], [base.Id, [layer(900)], kernelHash])
    // Later ordinary captures extend the flattened image one layer at a time.
    const next = { Id: image(3), Parent: flat.Id, Config: { User: "slice", Env: env }, RootFS: { Layers: [layer(900), layer(901)] } }
    recordCapturedImageProof(root, digest, flat, { ...container, Image: flat.Id }, next)
    assert.equal(readProtectedLayoutReceipt(root, next.Id.slice(7)).kernelHash, kernelHash)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("legacy captures accept a flattened image only as one parentless layer", async () => {
  const root = await privateRoot("flatten-legacy")
  try {
    const env = ["PATH=/usr/bin"]
    const layout = { sliceId: "chariox-slice-legacy", homeVolume: "chariox-slice-legacy-home", homeSource: "legacy" }
    const parent = { Id: image(4), RootFS: { Layers: [layer(1), layer(2)] } }
    const container = { Image: parent.Id, Config: { User: "slice", Env: env } }
    const flat = { Id: image(5), Parent: "", Config: { User: "slice", Env: env }, RootFS: { Layers: [layer(7)] } }
    assert.throws(() => recordLegacyImageProof(root, layout, parent, container, flat), /lineage is unverified/)
    assert.throws(() => recordLegacyImageProof(root, layout, parent, container,
      { ...flat, RootFS: { Layers: [layer(1), layer(2), layer(7)] }, Parent: parent.Id }, { flattened: true }), /lineage is unverified/)
    recordLegacyImageProof(root, layout, parent, container, flat, { flattened: true })
    assert.equal(readProtectedLayoutReceipt(root, flat.Id.slice(7)).ownerContainer, layout.sliceId)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

// Opt in with an existing slice image. The default Desktop save pauses a running
// slice, so a paused container must flatten exactly like a stopped one, and a
// running one is refused.
test("a paused container flattens with its changes and a running one is refused", {
  skip: !process.env.CHARIOX_SLICE_TEST_IMAGE,
  timeout: 1_800_000,
}, async () => {
  const docker = (...args) => execFileSync("docker", args, { encoding: "utf8", timeout: 600_000, maxBuffer: 16 * 1024 * 1024 }).trim()
  const name = `chariox-depth-paused-${process.pid}`, tag = `chariox-depth-test:${name}`
  try {
    docker("run", "-d", "--name", name, "--user", "0", "--entrypoint", "/bin/sh", process.env.CHARIOX_SLICE_TEST_IMAGE,
      "-c", "echo paused > /opt/chariox-depth-paused && exec sleep infinity")
    for (let n = 0; n < 50 && docker("exec", name, "sh", "-c", "cat /opt/chariox-depth-paused 2>/dev/null || true") !== "paused"; n++) {
      await new Promise((resolve) => setTimeout(resolve, 200))
    }
    await assert.rejects(flattenContainerImage({ container: name, image: tag }), /must be stopped or paused/)
    docker("pause", name)
    const captured = await captureContainerImage({ container: name, image: tag, maxLayers: 1 })
    assert.equal(captured.flattened, true)
    assert.equal(captured.image.RootFS.Layers.length, 1)
    assert.equal(docker("inspect", "--format", "{{.State.Paused}}", name), "true")
    assert.equal(docker("run", "--rm", "--network", "none", "--user", "0", "--entrypoint", "/bin/cat", tag, "/opt/chariox-depth-paused"), "paused")
  } finally {
    execFileSync("docker", ["rm", "-f", name], { stdio: "ignore" })
    execFileSync("docker", ["image", "rm", "-f", tag], { stdio: "ignore" })
  }
})

// Opt in with an existing slice image. Each round runs a container from the last
// saved image, changes its root filesystem and captures it, as nested
// restore/clone-then-save cycles do. A small bound forces repeated flattening.
test("repeated captures stay within the depth bound and keep every saved change", {
  skip: !process.env.CHARIOX_SLICE_TEST_IMAGE,
  timeout: 3_600_000,
}, async () => {
  const docker = (...args) => execFileSync("docker", args, { encoding: "utf8", timeout: 600_000, maxBuffer: 16 * 1024 * 1024 }).trim()
  const run = `chariox-depth-${process.pid}`
  const maxLayers = 3, rounds = 6
  const created = [], images = []
  let current = process.env.CHARIOX_SLICE_TEST_IMAGE
  const baseConfig = JSON.parse(docker("image", "inspect", "--format", "{{json .Config}}", current))
  const depths = []
  try {
    for (let round = 1; round <= rounds; round++) {
      const container = docker("create", "--name", `${run}-${round}`, "--label", `io.chariox.depth-test=${run}`,
        "--user", "0", "--entrypoint", "/bin/sh", current, "-c", `echo ${round} > /opt/chariox-depth-${round}`)
      created.push(container)
      docker("start", "-a", container)
      const tag = `chariox-depth-test:${run}-${round}`
      const captured = await captureContainerImage({ container, image: tag, maxLayers })
      images.push(tag)
      depths.push([captured.flattened, captured.image.RootFS.Layers.length])
      assert.ok(captured.image.RootFS.Layers.length <= maxLayers, `round ${round} is ${captured.image.RootFS.Layers.length} layers deep`)
      current = captured.image.Id
    }
    assert.deepEqual(depths.map(([, n]) => n), [1, 2, 3, 1, 2, 3])
    assert.deepEqual(depths.map(([flat]) => flat), [true, false, false, true, false, false])
    const saved = docker("run", "--rm", "--network", "none", "--user", "0", "--entrypoint", "/bin/sh", current,
      "-c", "cat /opt/chariox-depth-*")
    assert.deepEqual(saved.split(/\s+/), ["1", "2", "3", "4", "5", "6"])
    const finalConfig = JSON.parse(docker("image", "inspect", "--format", "{{json .Config}}", current))
    // The test containers override user and entrypoint; everything else is the base image's.
    assert.deepEqual(flattenedConfigMismatches(baseConfig, finalConfig).sort(), ["Cmd", "Entrypoint", "Labels", "User"].sort())
    assert.deepEqual(finalConfig.Env, baseConfig.Env)
    assert.equal(finalConfig.Labels["io.chariox.depth-test"], run)
  } finally {
    for (const container of created) execFileSync("docker", ["rm", "-f", container], { stdio: "ignore" })
    for (const tag of images.reverse()) execFileSync("docker", ["image", "rm", "-f", tag], { stdio: "ignore" })
  }
})
