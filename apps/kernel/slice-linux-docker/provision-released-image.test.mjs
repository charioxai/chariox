import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { spawnSync } from "node:child_process"
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const sliceDirectory = dirname(fileURLToPath(import.meta.url))
const revision = `sha256:${"a".repeat(64)}`
const layers = [`sha256:${"1".repeat(64)}`, `sha256:${"2".repeat(64)}`]
const archiveBytes = Buffer.from("not a real image, only bytes that docker load must receive unchanged\n".repeat(50000))

// The stub keeps what Docker would know: loaded tags, their layers and the calls made.
const dockerStub = `#!/usr/bin/env node
const { appendFileSync, existsSync, readFileSync, writeFileSync } = require("node:fs");
const args = process.argv.slice(2);
const statePath = process.env.CHARIOX_TEST_DOCKER_STATE;
const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, "utf8")) : { tags: {} };
const save = () => writeFileSync(statePath, JSON.stringify(state));
appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
const labels = { "io.chariox.relay-peer-protocol-version": "128", "io.chariox.runtime-source-revision": process.env.CHARIOX_TEST_IMAGE_REVISION,
  "io.chariox.selkies-version": "1", "io.chariox.selkies-source-revision": "f".repeat(40),
  "io.chariox.selkies-source": "https://github.com/selkies-project/selkies/commit/" + "f".repeat(40), "io.chariox.selkies-license": "MPL-2.0" };
if (args[0] === "info") { if (args.includes("--format")) console.log("x86_64"); process.exit(0); }
if (args[0] === "image" && args[1] === "inspect") {
  const image = state.tags[args[args.length - 1]];
  if (!image) process.exit(1);
  const format = args[3];
  if (format.includes("RootFS")) process.stdout.write(image.layers.map(layer => layer + ",").join(""));
  else { const label = /index \\.Config\\.Labels "([^"]+)"/.exec(format); console.log(labels[label[1]] ?? ""); }
  process.exit(0);
}
if (args[0] === "load") {
  const chunks = []; process.stdin.on("data", chunk => chunks.push(chunk));
  process.stdin.on("end", () => {
    const bytes = Buffer.concat(chunks);
    appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify({ loadedBytes: bytes.length }) + "\\n");
    state.tags["chariox-slice-release:fixture"] = { layers: JSON.parse(process.env.CHARIOX_TEST_LOADED_LAYERS) };
    save(); process.exit(0);
  });
  return;
}
if (args[0] === "tag") { state.tags[args[2]] = state.tags[args[1]]; save(); process.exit(0); }
if (args[0] === "rmi") { delete state.tags[args[1]]; save(); process.exit(0); }
if (args[0] === "buildx" || args[0] === "build") { appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, "BUILD\\n"); process.exit(1); }
console.error("unexpected Docker operation " + JSON.stringify(args)); process.exit(2);
`

async function install(root, { descriptor = {}, archive = archiveBytes, withImage = true } = {}) {
  const context = join(root, "slice-build-context")
  const provisionerDirectory = join(context, "apps/kernel/slice-linux-docker")
  await mkdir(dirname(provisionerDirectory), { recursive: true })
  await cp(sliceDirectory, provisionerDirectory, { recursive: true, filter: source => !source.includes("/node_modules") })
  await mkdir(join(context, "apps/kernel/src/transport"), { recursive: true })
  await writeFile(join(context, "apps/kernel/src/transport/relay_peer.rs"), "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 128;\n")
  if (withImage) {
    const image = join(root, "slice-image")
    await mkdir(image, { mode: 0o755 })
    await writeFile(join(image, "chariox-slice-linux.tar.gz"), archive, { mode: 0o644 })
    await writeFile(join(image, "slice-image.json"), JSON.stringify({
      schema: "chariox.slice-image.v1", architecture: "amd64", tag: "chariox-slice-release:fixture",
      runtimeSourceRevision: revision, relayPeerProtocolVersion: "128", rootfsLayers: layers,
      archive: { name: "chariox-slice-linux.tar.gz", size: archiveBytes.length, sha256: createHash("sha256").update(archiveBytes).digest("hex") },
      ...descriptor,
    }), { mode: 0o644 })
  }
  await writeFile(join(root, "docker"), dockerStub, { mode: 0o700 })
  return join(provisionerDirectory, "provision-linux-docker-slice.sh")
}

function run(root, provisioner, { loadedLayers = layers } = {}) {
  const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("CHARIOX_SLICE_")))
  return spawnSync("bash", [provisioner, "build-image"], {
    encoding: "utf8", timeout: 60_000,
    env: {
      ...environment, PATH: `${root}:${environment.PATH}`, TMPDIR: root,
      CHARIOX_TEST_DOCKER_LOG: join(root, "docker.log"), CHARIOX_TEST_DOCKER_STATE: join(root, "docker.json"),
      CHARIOX_TEST_IMAGE_REVISION: revision, CHARIOX_TEST_LOADED_LAYERS: JSON.stringify(loadedLayers),
      CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: revision, CHARIOX_SLICE_NAME: "chariox-released-image-fixture",
    },
  })
}

const dockerCalls = async root => (await readFile(join(root, "docker.log"), "utf8").catch(() => "")).trim().split("\n").filter(Boolean)
const withRoot = async body => {
  const root = await mkdtemp(join(tmpdir(), "chariox-released-image-"))
  try { await body(root) } finally { await rm(root, { recursive: true, force: true }) }
}

test("a release install loads its verified image and never builds", () => withRoot(async root => {
  const result = run(root, await install(root))
  assert.equal(result.status, 0, result.stderr)
  const calls = await dockerCalls(root)
  assert.ok(!calls.includes("BUILD"), "a release install must not build the image")
  assert.ok(calls.includes(JSON.stringify({ loadedBytes: archiveBytes.length })), "docker load receives the archive bytes")
  assert.ok(calls.includes(JSON.stringify(["tag", "chariox-slice-release:fixture", "chariox-slice-linux:0.1.0"])))
  const mb = archiveBytes.length >> 20
  assert.match(result.stderr, new RegExp(`progress: Verifying the runtime image \\d+ MB of ${mb} MB \\(100%\\)`))
  assert.match(result.stderr, new RegExp(`progress: Loading the runtime image ${mb} MB of ${mb} MB \\(100%\\)`))
}))

test("an image that is already loaded and matches is not loaded again", () => withRoot(async root => {
  const provisioner = await install(root)
  assert.equal(run(root, provisioner).status, 0)
  await rm(join(root, "docker.log"))
  const result = run(root, provisioner)
  assert.equal(result.status, 0, result.stderr)
  assert.ok(!(await dockerCalls(root)).some(call => call.includes("loadedBytes")))
}))

test("an archive that differs from the signed digest never reaches docker load", () => withRoot(async root => {
  const tampered = Buffer.from(archiveBytes)
  tampered[100] ^= 1
  const result = run(root, await install(root, { archive: tampered }))
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /does not match the SHA-256 the signed release lists/)
  const calls = await dockerCalls(root)
  assert.ok(!calls.includes("BUILD") && !calls.some(call => call.includes("loadedBytes")))
}))

test("a loaded image with other layers is removed and refused", () => withRoot(async root => {
  const result = run(root, await install(root), { loadedLayers: [`sha256:${"9".repeat(64)}`] })
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /does not match the signed release; it was removed/)
  const calls = await dockerCalls(root)
  assert.ok(calls.includes(JSON.stringify(["rmi", "chariox-slice-release:fixture"])))
  assert.ok(!calls.includes("BUILD"))
}))

test("an image built from other sources fails loudly instead of building locally", () => withRoot(async root => {
  const result = run(root, await install(root, { descriptor: { runtimeSourceRevision: `sha256:${"b".repeat(64)}` } }))
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /built from different runtime sources/)
  assert.ok(!(await dockerCalls(root)).includes("BUILD"))
}))

test("a world-writable archive is refused", () => withRoot(async root => {
  const provisioner = await install(root)
  const { chmodSync } = await import("node:fs")
  chmodSync(join(root, "slice-image/chariox-slice-linux.tar.gz"), 0o666)
  const result = run(root, provisioner)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /writable by other users/)
}))

test("a source checkout without a shipped image still builds locally", () => withRoot(async root => {
  const result = run(root, await install(root, { withImage: false }))
  assert.notEqual(result.status, 0)
  assert.ok((await dockerCalls(root)).includes("BUILD"))
}))
