import { test } from "node:test"
import assert from "node:assert/strict"
import { PRIVATE_ROOT, PRIVATE_ENVIRONMENT, verifyProtectedCaptureLayout, requireSupportedHomeEntries } from "../apps/kernel/slice-linux-docker/protected-layout.mjs"

const digest = `sha256:${"a".repeat(64)}`
function fixture() {
  const receipt = { version: 1, containerId: "synthetic", imageId: digest, baseImageId: digest,
    privateHostRoot: "/protected/synthetic", homeVolume: "synthetic-home" }
  const inspect = { Id: "synthetic", Image: digest, Mounts: [
    { Type: "bind", Source: receipt.privateHostRoot, Destination: PRIVATE_ROOT, RW: true },
    { Type: "volume", Name: receipt.homeVolume, Destination: "/home/slice", RW: true },
    { Type: "bind", Source: `${receipt.privateHostRoot}/nssdb`, Destination: "/home/slice/.local/share/pki/nssdb", RW: true },
  ], Config: { Env: ["HOME=/home/slice", ...Object.entries(PRIVATE_ENVIRONMENT).map(([k,v]) => `${k}=${v}`)] } }
  return { inspect, receipt }
}
const verify = ({ inspect, receipt }, trusted = new Set([digest])) => verifyProtectedCaptureLayout(inspect, receipt, trusted)
test("Room audio uses only the fixed non-home PulseAudio runtime path", () => {
  const supported = fixture()
  supported.inspect.Config.Env.push("PULSE_RUNTIME_PATH=/tmp/chariox-pulse-runtime")
  assert.doesNotThrow(() => verify(supported))
  for (const value of ["/home/slice/.config/pulse/runtime", "/var/lib/chariox/slice-private/pulse", "/tmp/other-pulse-runtime"]) {
    const unsupported = fixture()
    unsupported.inspect.Config.Env.push(`PULSE_RUNTIME_PATH=${value}`)
    assert.throws(() => verify(unsupported))
  }
})

test("verified actual topology accepts only the protected roots", () => assert.deepEqual(verify(fixture()), { privateHostRoot: "/protected/synthetic", homeVolume: "synthetic-home" }))
test("standard image public defaults coexist with the protected container environment", () => {
  const f = fixture()
  f.inspect.Config.Env.push("NODE_VERSION=22.17.1", "YARN_VERSION=1.22.22", "PATH=/usr/local/bin:/usr/bin:/bin",
    "DEBIAN_FRONTEND=noninteractive", "CHARIOX_SLICE_ROOT=/opt/chariox-slice",
    "CHARIOX_SLICE_SELKIES_BIN=/opt/chariox-selkies/bin/selkies", "CHARIOX_SLICE_DISPLAY_SERVER=Xorg")
  assert.doesNotThrow(() => verify(f))
  for (const value of ["NODE_VERSION=synthetic-private-token", "YARN_VERSION=", "CHARIOX_SLICE_DISPLAY_SERVER=unsupported"]) {
    const bad = fixture(); bad.inspect.Config.Env.push(value)
    assert.throws(() => verify(bad))
  }
})
test("forged labels cannot replace mounts or trusted base proof", () => {
  const f = fixture(); f.inspect.Config.Labels = { protected: "true" }; f.inspect.Mounts = []
  assert.throws(() => verify(f)); assert.throws(() => verify(fixture(), new Set()))
})
test("wrong source, nested overlay and missing NSS separation refuse", () => {
  for (const change of [f => f.inspect.Mounts[0].Source = "/different", f => f.inspect.Mounts.pop(), f => f.inspect.Mounts.push({Destination: `${PRIVATE_ROOT}/kernel`, Type: "volume"})]) {
    const f = fixture(); change(f); assert.throws(() => verify(f))
  }
})
test("token environment, duplicate env and unsafe host path refuse without secret output", () => {
  for (const change of [f => f.inspect.Config.Env.push("TOKEN=synthetic-private-sentinel"), f => f.inspect.Config.Env.push("HOME=/other"), f => f.receipt.privateHostRoot = "/protected/../other"]) {
    const f = fixture(); change(f); assert.throws(() => verify(f), e => !e.message.includes("sentinel"))
  }
})
test("runtime overlays cannot replace the trusted initializer through a parent or leaf bind", () => {
  for (const destination of ["/", "/opt", "/opt/chariox-slice", "/opt/chariox-slice/bin/chariox-kernel"]) {
    const value = fixture()
    value.inspect.Mounts.push({Type: "bind", Source: "/synthetic-untrusted", Destination: destination, RW: false})
    assert.throws(() => verify(value))
  }
})
test("known credential roots refuse while browser/workspace data stays supported", () => {
  for (const path of [".codex/auth.json", ".chariox/kernels/id/identity.json", ".ssh/key", "../private"]) assert.throws(() => requireSupportedHomeEntries([path]))
  assert.doesNotThrow(() => requireSupportedHomeEntries([".chariox/browser/chromium/Cookies", "Downloads/synthetic.txt"]))
})

test("protected GitHub config is a private environment path, never a captured home alias", async () => {
  const {readFile, mkdtemp, mkdir, writeFile, rm, lstat} = await import("node:fs/promises")
  const {spawnSync} = await import("node:child_process")
  const root = await mkdtemp(join(process.env.HOME, ".chariox-github-capture-test-"))
  try {
    const home = join(root, "home"), privateHome = join(root, "private"), bin = join(root, "bin")
    for (const path of [home, privateHome, bin]) await mkdir(path, {mode: 0o700})
    await writeFile(join(bin, "gh"), '#!/bin/sh\nmkdir -p "$GH_CONFIG_DIR"\nprintf synthetic-auth-marker > "$GH_CONFIG_DIR/hosts.yml"\n', {mode: 0o700})
    const source = await readFile(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url), "utf8")
    const body = source.split("import_github_auth() {")[1].split("remove_github_auth() {")[0]
    const command = body.slice(body.indexOf('  run_with_file_stdin_timeout 90'), body.indexOf('  rm -f "$token_tmp"', body.indexOf('  run_with_file_stdin_timeout 90')))
      .replaceAll("/home/slice", home)
    const wrapper = `run_with_file_stdin_timeout() { local command="\${@: -1}"; PATH="$SYNTHETIC_BIN:/usr/bin:/bin" bash -c "$command"; }\n${command}`
    const result = spawnSync("bash", ["-c", wrapper], {encoding:"utf8", env:{...process.env, SYNTHETIC_BIN:bin,
      SLICE_PRIVATE_HOST_ROOT:privateHome, SLICE_PROVIDER_HOME:privateHome, SLICE_NAME:"synthetic", SLICE_GITHUB_HOST:"synthetic.invalid", GH_CONFIG_DIR:join(privateHome,".config/gh"), token_tmp:"unused-synthetic"}})
    assert.equal(result.status, 0, result.stderr)
    assert.equal((await lstat(join(privateHome,".config/gh/hosts.yml"))).isFile(),true)
    await assert.rejects(lstat(join(home,".config/gh")), {code:"ENOENT"})
    assert.doesNotThrow(() => requireSupportedHomeEntries([".config", ".gitconfig"]))
    assert.equal(PRIVATE_ENVIRONMENT.GH_CONFIG_DIR, `${PRIVATE_ROOT}/provider-home/.config/gh`)
    assert.deepEqual(verify(fixture()), {privateHostRoot:"/protected/synthetic",homeVolume:"synthetic-home"})
    const archive = spawnSync("tar", ["-cf", "-", "-C", home, "."])
    assert.equal(archive.status,0,archive.stderr.toString())
    assert.equal(archive.stdout.includes(Buffer.from("synthetic-auth-marker")),false)
    const checked = spawnSync("python3", [new URL("../apps/kernel/slice-linux-docker/validate-home-archive.py", import.meta.url).pathname], {input:archive.stdout,encoding:"utf8"})
    assert.equal(checked.status,0,checked.stderr)
  } finally { await rm(root,{recursive:true}) }
})

import { mkdtempSync, rmSync, symlinkSync, unlinkSync } from "node:fs"
import { join } from "node:path"
import { preparePrivateHostRoot } from "../apps/kernel/slice-linux-docker/protected-host-root.mjs"
test("private host roots never overwrite an existing slice or repair a missing root", () => {
  // tmpdir may have a writable ancestor, which deliberately fails the production
  // contract. Use the owned non-writable home ancestor for this synthetic test.
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-test-"))
  try {
    const uid = process.getuid()
    const path = preparePrivateHostRoot(root, "synthetic", uid, true)
    assert.equal(preparePrivateHostRoot(root, "synthetic", uid, false), path)
    assert.throws(() => preparePrivateHostRoot(root, "synthetic", uid, true))
    assert.throws(() => preparePrivateHostRoot(root, "missing", uid, false))
    symlinkSync(path, join(root, "alias"))
    assert.throws(() => preparePrivateHostRoot(root, "alias", uid, false))
  } finally { rmSync(root, { recursive: true }) }
})

import { existsSync, lstatSync, writeFileSync, readFileSync } from "node:fs"
import { streamArchiveToProtectedSink } from "../apps/kernel/slice-linux-docker/protected-archive-stream.mjs"
test("archive streams only into protected durable sink and preserves prior generations", async () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-stream-test-"))
  try {
    const previous = join(root, "previous.tar.zst")
    writeFileSync(previous, "synthetic prior generation", {mode: 0o600})
    const path = join(root, "next.tar.zst")
    const result = await streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic browser data')"], path, reserveBytes: 0})
    assert.equal(result.sizeBytes, 22)
    assert.equal(lstatSync(path).mode & 0o777, 0o600)
    assert.equal(readFileSync(previous, "utf8"), "synthetic prior generation")
    await assert.rejects(streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic interrupted data');process.exit(1)"],
      path: join(root, "failed.tar.zst"), reserveBytes: 0}))
    assert.equal(existsSync(join(root, "failed.tar.zst")), false)
    assert.equal(readFileSync(previous, "utf8"), "synthetic prior generation")
    await assert.rejects(streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic too-large data')"],
      path: join(root, "oversize.tar.zst"), maxBytes: 2, reserveBytes: 0}))
    assert.equal(existsSync(join(root, "oversize.tar.zst")), false)
  } finally { rmSync(root, {recursive: true}) }
})

import { writeProtectedLayoutReceipt, readProtectedLayoutReceipt, requireRetainedRuntimeIdentity } from "../apps/kernel/slice-linux-docker/protected-layout-store.mjs"
import { mkdirSync, chmodSync } from "node:fs"
test("trusted host receipts and retained identity cannot be replaced by labels or missing files", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-receipt-test-"))
  try {
    const receipt = {version: 1, sliceId: "synthetic", containerId: "synthetic-container"}
    writeProtectedLayoutReceipt(root, "synthetic", receipt)
    assert.deepEqual(readProtectedLayoutReceipt(root, "synthetic"), receipt)
    assert.throws(() => readProtectedLayoutReceipt(root, "../synthetic"))
    const privateRoot = preparePrivateHostRoot(root, "private", process.getuid(), true)
    const identity = "kernel/state/daemon/identity.json"
    assert.throws(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
    writeFileSync(join(privateRoot, identity), "synthetic sentinel, no private key", {mode: 0o600})
    assert.doesNotThrow(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
    chmodSync(join(privateRoot, identity), 0o644)
    assert.throws(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
  } finally { rmSync(root, {recursive: true}) }
})

import { recordManagedImageProof, requireManagedImageProof } from "../apps/kernel/slice-linux-docker/protected-image-proof.mjs"
test("managed image proof binds actual immutable image ID to signed source context", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-image-test-"))
  try {
    const source = `sha256:${"b".repeat(64)}`
    const image = {Id: digest, Config: {User: "slice"}, RootFS: {Layers: [`sha256:${"c".repeat(64)}`]}}
    assert.throws(() => requireManagedImageProof(root, source, digest))
    recordManagedImageProof(root, source, image)
    assert.equal(requireManagedImageProof(root, source, digest), digest)
    assert.throws(() => requireManagedImageProof(root, `sha256:${"d".repeat(64)}`, digest))
    assert.throws(() => recordManagedImageProof(root, "self-asserted", image))
    assert.throws(() => recordManagedImageProof(root, source, {...image, Config: {User: "root"}}))
  } finally { rmSync(root, {recursive: true}) }
})

import { createManagedLayoutController, findRetainedCaptureOrigin } from "../apps/kernel/slice-linux-docker/protected-managed-layout.mjs"
import { verifyNamespaceEntryDocuments } from "../apps/kernel/slice-linux-docker/protected-namespace-entry.mjs"
test("daemon namespace proof pins actual maps and namespace identity without treating control UID0 as host root", () => {
  const namespaces = Object.fromEntries(["user", "mnt", "net"].map((name, index) => [name, {dev: "4", ino: String(100 + index)}]))
  const uidMap = "0 997 1\n1 231072 65536\n"
  const gidMap = "0 998 1\n1 231072 65536\n"
  const receipt = {version: 1, daemonUid: 997, daemonGid: 998, dataUid: 1001,
    hostDataUid: 232072, uidMap, gidMap, namespaces, ancestors: [
      ["/", 0, 0o755], ["/var", 0, 0o755], ["/var/lib", 0, 0o755],
      ["/var/lib/chariox-docker", 997, 0o700], ["/var/lib/chariox-docker/private-layout", 997, 0o711],
      ["/var/lib/chariox-slice-share", 0, 0o710], ["/var/lib/chariox-slice-share/.broker-private", 0, 0o711],
      ["/var/lib/chariox-slice-share/.broker-private/artifacts", 997, 0o700],
    ].map(([path,hostUid,mode],index) => ({path,hostUid,mode,dev:"4",ino:String(200+index)}))}
  const current = {uid: 0, gid: 0, uidMap, gidMap, namespaces,
    subuids: "chariox-docker:231072:65536", subgids: "chariox-docker:231072:65536"}
  assert.equal(verifyNamespaceEntryDocuments(receipt, current).dataUid, 1001)
  for (const mutation of [value => value.uid = 997, value => value.uidMap = "0 0 4294967295",
    value => value.namespaces.user.ino = "foreign", value => value.subuids = "chariox-docker:300000:65536"]) {
    const changed = structuredClone(current); mutation(changed)
    assert.throws(() => verifyNamespaceEntryDocuments(receipt, changed))
  }
  assert.throws(() => verifyNamespaceEntryDocuments({...receipt, dataUid: 232072}, current))
})
import { createHash } from "node:crypto"
test("restore resolution binds the original transaction artifact independently of the replacement container", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-restore-origin-test-"))
  try {
    chmodSync(root, 0o711)
    const container = "chariox-slice-origin"
    const archiveDigest = "a".repeat(64)
    const imageId = `sha256:${"b".repeat(64)}`
    const originRoot = join(root, "capture-origins")
    const receiptRoot = join(root, "receipts")
    mkdirSync(originRoot, {mode: 0o700}); mkdirSync(receiptRoot, {mode: 0o700})
    const key = createHash("sha256").update(`older-captured-container\0${archiveDigest}`).digest("hex")
    const origin = {version: 1, sliceId: key, container, containerId: "older-captured-container",
      homeVolume: `${container}-older-home`, digest: archiveDigest}
    writeProtectedLayoutReceipt(originRoot, key, origin)
    const pendingPath = join(originRoot, `.${"d".repeat(64)}.123.pending`)
    writeFileSync(pendingPath, "synthetic interrupted public metadata", {mode: 0o600})
    assert.deepEqual(findRetainedCaptureOrigin(originRoot, container, archiveDigest), origin)
    assert.equal(readFileSync(pendingPath, "utf8"), "synthetic interrupted public metadata")
    chmodSync(pendingPath, 0o644)
    assert.throws(() => findRetainedCaptureOrigin(originRoot, container, archiveDigest))
    chmodSync(pendingPath, 0o600)
    const generations = createHomeGenerationStore(root)
    const pending = generations.begin({container, oldHomeVolume: `${container}-home`,
      oldContainerId: "recent-container", archiveDigest, imageId, targetOrigin: origin})
    const parameters = {container, token: pending.token, volume: pending.newHomeVolume, digest: archiveDigest}
    generations.complete(parameters); generations.publish(parameters)
    writeProtectedLayoutReceipt(receiptRoot, container, {version: 1, sliceId: container,
      containerId: "replacement-container", homeVolume: pending.newHomeVolume})
    const controller = createManagedLayoutController({root, sourceDigest: imageId,
      dataOwner: process.getuid(), docker: () => { throw new Error("resolution must not run Docker") }})
    assert.throws(() => controller.resolveRestore(container, "c".repeat(64)))
    assert.equal(generations.read(container).phase, "published")
    controller.resolveRestore(container, archiveDigest)
    assert.equal(generations.read(container).phase, "resolved")
    assert.equal(generations.read(container).oldHomeVolume, `${container}-home`)
    assert.equal(readProtectedLayoutReceipt(originRoot, key).containerId, "older-captured-container")
  } finally { rmSync(root, {recursive: true}) }
})
test("fresh controller binds exact image/mount receipt and refuses missing same-slice identity", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-layout-controller-test-"))
  try {
    const root = join(parent, "durable")
    const source = `sha256:${"b".repeat(64)}`
    const environment = {CHARIOX_SLICE_NAME: "chariox-slice-synthetic", CHARIOX_SLICE_ID: "slice-synthetic"}
    let info = null
    const docker = args => {
      if (args[0] === "ps") return {status: 0, stdout: info ? `${environment.CHARIOX_SLICE_NAME}\n` : ""}
      if (args[0] === "volume") return {status: 0, stdout: ""}
      if (args[0] === "container") return {status: 0, stdout: JSON.stringify([info])}
      throw new Error("unexpected synthetic Docker operation")
    }
    const controller = createManagedLayoutController({root, sourceDigest: source, docker, dataOwner: process.getuid()})
    assert.throws(() => controller.prepare("provision", {...environment, CHARIOX_SLICE_NAME: "../foreign"}))
    const privateRoot = controller.prepare("provision", environment)
    assert.throws(() => controller.prepare("provision", environment), "existing retained root cannot be silently initialized again")
    recordManagedImageProof(controller.imageRoot, source, {Id: digest, Config: {User: "slice"}, RootFS: {Layers: [digest]}})
    const directory = join(privateRoot, "kernel/kernels/synthetic-kernel")
    mkdirSync(directory, {mode: 0o700})
    const identity = join(directory, "identity.json")
    writeFileSync(identity, "synthetic identity sentinel, no private key", {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/kernels/registry.json"), "synthetic registry sentinel", {mode: 0o600})
    info = fixture().inspect
    info.Id = "synthetic-container"
    info.Mounts[0].Source = privateRoot
    info.Mounts[1].Name = `${environment.CHARIOX_SLICE_NAME}-home`
    info.Mounts[2].Source = `${privateRoot}/nssdb`
    environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT = privateRoot
    assert.throws(() => controller.complete(environment), "missing first-use restoration proof cannot enable capture")
    assert.throws(() => controller.preflight(environment.CHARIOX_SLICE_NAME))
    assert.throws(() => controller.prepare("provision", environment), "an unreceipted protected container cannot fall back to legacy boot")
    const foreignOwnerController = createManagedLayoutController({root, sourceDigest: source, docker, dataOwner: process.getuid() + 1})
    assert.throws(() => foreignOwnerController.complete(environment))
    rmSync(identity)
    assert.throws(() => controller.complete(environment))
    assert.equal(existsSync(identity), false, "missing identity must not be generated")
    assert.equal(existsSync(privateRoot), true, "retained private root must remain")
  } finally { rmSync(parent, {recursive: true}) }
})

test("provider auth layout retains legacy paths and refuses unreceipted private mounts", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-provider-layout-test-"))
  try {
    const container = "chariox-slice-synthetic-auth"
    let info = {Id:"synthetic", Mounts:[], Config:{Env:[]}}
    const docker = args => args[0] === "ps" ? {status:0,stdout:`${container}\n`} : {status:0,stdout:JSON.stringify([info])}
    const controller = createManagedLayoutController({root,sourceDigest:`sha256:${"b".repeat(64)}`,docker,dataOwner:process.getuid()})
    assert.equal(controller.providerAuthProtected(container),false)
    info.Mounts.push({Destination:PRIVATE_ROOT})
    assert.throws(() => controller.providerAuthProtected(container))
    info = {Id:"synthetic",Mounts:[],Config:{Env:[`CHARIOX_SLICE_PRIVATE_ROOT=${PRIVATE_ROOT}`]}}
    assert.throws(() => controller.providerAuthProtected(container))
    info = null
    assert.throws(() => controller.providerAuthProtected(container))
  } finally { rmSync(root,{recursive:true}) }
})

test("provider auth layout broker request requires the exact owned-container contract", () => {
  const path = new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs",import.meta.url).pathname
  const valid = {kind:"provider_auth_layout",container:"chariox-slice-synthetic-auth"}
  const check = request => spawnSync(process.execPath,[path,"--validate-request"],{input:JSON.stringify(request),encoding:"utf8"})
  assert.equal(check(valid).status,0)
  for (const request of [{...valid,container:"foreign"},{...valid,path:"/private"},{...valid,container:"chariox-helper"}]) {
    assert.notEqual(check(request).status,0)
  }
})

import { retainFreshIdentity } from "../apps/kernel/slice-linux-docker/protected-identity-retention.mjs"
test("first-use retention refuses missing or invalid synthetic identity without starting a runtime", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-retention-metadata-test-"))
  try {
    const privateRoot = preparePrivateHostRoot(parent, "private-retention", process.getuid(), true)
    const backupRoot = join(parent, "backups")
    mkdirSync(backupRoot, {mode: 0o700})
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}))
    assert.equal(existsSync(join(backupRoot, "synthetic")), false)
    const kernel = join(privateRoot, "kernel/kernels/synthetic-kernel")
    mkdirSync(kernel, {mode: 0o700})
    writeFileSync(join(kernel, "identity.json"), JSON.stringify({daemon_id: "synthetic-kernel", relay_public_key: "invalid synthetic sentinel", relay_private_key: "not a key"}), {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/kernels/registry.json"), "synthetic registry", {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/machine/identity.json"), "synthetic machine", {mode: 0o600})
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}))
    assert.equal(existsSync(join(backupRoot, "synthetic.json")), false, "invalid identity cannot publish first-use proof")
    assert.equal(existsSync(join(kernel, "identity.json")), true, "original identity remains untouched")
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}), "interrupted backup must not be replaced")
  } finally { rmSync(parent, {recursive: true}) }
})

import { mappedSliceOwner, SLICE_CONTAINER_UID } from "../apps/kernel/slice-linux-docker/protected-rootless-owner.mjs"
test("managed host owner derives from the actual rootless mapping rather than container UID", () => {
  const metadata = {daemonUid: 987, daemonGid: 987, processUid: 987,
    uidMap: "0 987 1\n1 231072 65536\n", gidMap: "0 987 1\n1 231072 65536\n",
    subuids: "chariox-docker:231072:65536\n", subgids: "chariox-docker:231072:65536\n"}
  assert.equal(SLICE_CONTAINER_UID, 1001)
  assert.equal(mappedSliceOwner(metadata), 232072)
  assert.notEqual(mappedSliceOwner(metadata), SLICE_CONTAINER_UID)
  assert.throws(() => mappedSliceOwner({...metadata, processUid: 988}))
  assert.throws(() => mappedSliceOwner({...metadata, uidMap: "0 0 4294967295\n"}))
  assert.throws(() => mappedSliceOwner({...metadata, gidMap: "0 987 1\n1 331072 65536\n"}))
  assert.throws(() => mappedSliceOwner({...metadata, subuids: "chariox-docker:231072:65536\nchariox-docker:331072:65536\n"}))
})

import { ensureFirstBootRetention, verifyFirstBootTopology } from "../apps/kernel/slice-linux-docker/protected-first-boot.mjs"
import { createHomeGenerationStore } from "../apps/kernel/slice-linux-docker/protected-home-generation.mjs"
import { spawnSync } from "node:child_process"
test("process interruption during staged extraction cannot boot partial data or replace the prior home", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-generation-interruption-test-"))
  try {
    chmodSync(root, 0o711)
    const prior = join(root, "prior-home")
    mkdirSync(prior, {mode: 0o700})
    writeFileSync(join(prior, "browser-state"), "synthetic retained browser state")
    const module = new URL("../apps/kernel/slice-linux-docker/protected-home-generation.mjs", import.meta.url).href
    const child = spawnSync(process.execPath, ["--input-type=module", "-e", `
      import {createHomeGenerationStore} from ${JSON.stringify(module)};
      import {mkdirSync,writeFileSync} from 'node:fs';
      import {join} from 'node:path';
      const root=process.argv[1];
      const pending=createHomeGenerationStore(root).begin({container:'chariox-slice-fault',
        oldHomeVolume:'chariox-slice-fault-home',oldContainerId:'old-container',
        archiveDigest:'a'.repeat(64),imageId:'sha256:'+ 'b'.repeat(64)});
      const staged=join(root,pending.newHomeVolume);
      mkdirSync(staged,{mode:0o700});
      writeFileSync(join(staged,'partial-browser-state'),'synthetic incomplete extraction');
      process.kill(process.pid,'SIGKILL');
    `, root], {timeout: 5000})
    assert.equal(child.signal, "SIGKILL")
    const restarted = createHomeGenerationStore(root)
    const pending = restarted.read("chariox-slice-fault")
    assert.equal(pending.phase, "preparing")
    assert.throws(() => restarted.requireReady({container: pending.container, token: pending.token,
      volume: pending.newHomeVolume, digest: pending.archiveDigest}))
    assert.equal(readFileSync(join(prior, "browser-state"), "utf8"), "synthetic retained browser state")
    assert.equal(pending.oldHomeVolume, "chariox-slice-fault-home")
    assert.throws(() => restarted.resolve(pending.container, pending.newHomeVolume))
    const rollback = restarted.prepareRollback({container: pending.container, archiveDigest: "c".repeat(64),
      imageId: `sha256:${"b".repeat(64)}`, origin: {container: pending.container,
        homeVolume: pending.oldHomeVolume, containerId: pending.oldContainerId, digest: "c".repeat(64)}})
    restarted.publish({container: rollback.container, token: rollback.token,
      volume: rollback.newHomeVolume, digest: rollback.archiveDigest})
    assert.equal(restarted.read(pending.container).failedHomeVolume, pending.newHomeVolume)
    restarted.resolve(pending.container, pending.oldHomeVolume)
    assert.equal(restarted.read(pending.container).phase, "resolved")
    assert.equal(existsSync(join(root, pending.newHomeVolume, "partial-browser-state")), true,
      "publication must retain interrupted data references until separate retirement")
  } finally { rmSync(root, {recursive: true}) }
})
test("interrupted restore generations cannot become ready or discard the previous home reference", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-generation-metadata-test-"))
  try {
    chmodSync(root, 0o711)
    const store = createHomeGenerationStore(root)
    const container = "chariox-slice-synthetic"
    const digest = "a".repeat(64)
    const pending = store.begin({container, oldHomeVolume: `${container}-home`, archiveDigest: digest, imageId: `sha256:${digest}`})
    const parameters = {container, token: pending.token, volume: pending.newHomeVolume, digest}
    assert.throws(() => store.requireReady(parameters))
    assert.throws(() => store.begin({container, oldHomeVolume: `${container}-home`, archiveDigest: digest, imageId: `sha256:${digest}`}))
    assert.throws(() => store.complete({...parameters, token: "foreign"}))
    assert.equal(store.read(container).oldHomeVolume, `${container}-home`)
    assert.equal(store.read(container).phase, "preparing")
    store.complete(parameters)
    assert.equal(store.requireReady(parameters).oldHomeVolume, `${container}-home`)
    store.publish(parameters)
    assert.equal(store.read(container).phase, "published")
    assert.equal(store.read(container).oldHomeVolume, `${container}-home`, "publication alone must not retire rollback data")
  } finally { rmSync(root, {recursive: true}) }
})
test("real first-boot preflight passes the declared host root into topology verification", () => {
  const {inspect, receipt} = fixture()
  assert.deepEqual(verifyFirstBootTopology(inspect, receipt.privateHostRoot, receipt.homeVolume),
    {privateHostRoot: receipt.privateHostRoot, homeVolume: receipt.homeVolume})
  assert.throws(() => verifyFirstBootTopology(inspect, "/different", receipt.homeVolume))
})
import { parseRuntimeHash, verifyRuntimeMetadata } from "../apps/kernel/slice-linux-docker/protected-runtime-proof.mjs"
test("trusted initializer requires an immutable ordinary-user runtime path and exact public hash", () => {
  const paths = ["/", "/opt", "/opt/chariox-slice", "/opt/chariox-slice/bin", "/opt/chariox-slice/bin/chariox-kernel"]
  const rows = paths.map((path, index) => `0|755|${index === 4 ? "regular file" : "directory"}|${path}`)
  assert.doesNotThrow(() => verifyRuntimeMetadata(rows.join("\n")))
  for (const mutate of [rows => { rows[2] = rows[2].replace("0|", "1001|") },
    rows => { rows[3] = rows[3].replace("755", "775") },
    rows => { rows[4] = rows[4].replace("regular file", "symbolic link") }]) {
    const altered = [...rows]; mutate(altered); assert.throws(() => verifyRuntimeMetadata(altered.join("\n")))
  }
  assert.equal(parseRuntimeHash(`${"a".repeat(64)}  ${paths[4]}\n`), "a".repeat(64))
  assert.throws(() => parseRuntimeHash(`${"a".repeat(64)}  /different\n`))
})
import { validateBootSelection } from "../apps/kernel/slice-linux-docker/protected-identity-retention.mjs"
test("retention pins the exact registry endpoint, machine and selected identity before boot", () => {
  const identity = {kernel_id: "synthetic-kernel", host: "127.0.0.1", port: 43119,
    relay_public_key: "synthetic-public", relay_private_key: "synthetic-private-sentinel"}
  const input = {identity, registry: {version: 1, machine_id: "synthetic-machine", kernels: {"127.0.0.1:43119": {...identity}}},
    machine: {machine_id: "synthetic-machine"}, host: "127.0.0.1", port: 43119}
  const expected = validateBootSelection(input)
  assert.doesNotThrow(() => validateBootSelection({...input, expected}))
  const mutations = [
    value => { value.registry = {} },
    value => { value.registry.kernels = {} },
    value => { value.registry.machine_id = "foreign-machine" },
    value => { value.machine.machine_id = "foreign-machine" },
    value => { value.registry.kernels["127.0.0.1:43119"].kernel_id = "foreign-kernel" },
    value => { value.identity.relay_private_key = "different-synthetic-sentinel" },
    value => { value.registry.kernels["127.0.0.1:43120"] = {...identity, port: 43120} },
    value => { value.expected = {...expected, relayPublicKey: "foreign-public"} },
  ]
  for (const mutate of mutations) {
    const value = structuredClone({...input, expected})
    mutate(value)
    assert.throws(() => validateBootSelection(value), /retention proof is unavailable/)
  }
  assert.deepEqual(validateBootSelection(input), expected, "rejection must not alter retained inputs")
})
test("first-boot barrier invokes only offline preparation and refuses partial identity without retry generation", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-first-boot-metadata-test-"))
  try {
    const privateRoot = preparePrivateHostRoot(parent, "private-boot", process.getuid(), true)
    const backupRoot = join(parent, "backups")
    mkdirSync(backupRoot, {mode: 0o700})
    const calls = []
    const parameters = {privateRoot, backupRoot, sliceId: "synthetic", container: "chariox-slice-synthetic", port: 43119,
      dataOwner: process.getuid(), docker: args => { calls.push(args); return {status: 1} }}
    assert.throws(() => ensureFirstBootRetention(parameters))
    assert.equal(calls.length, 1)
    assert.equal(calls[0].at(-2), "--prepare-protected-slice-identity")
    assert.equal(calls[0][2], "1001", "container UID is distinct from host owner")
    const started = join(backupRoot, "synthetic.initialization-started.json")
    assert.equal(lstatSync(started).mode & 0o777, 0o600)
    assert.throws(() => ensureFirstBootRetention(parameters))
    assert.equal(calls.length, 1, "missing identity and receipt after a failed first attempt must not generate a substitute")
    writeFileSync(join(privateRoot, "kernel/machine/identity.json"), "synthetic incomplete sentinel", {mode: 0o600})
    assert.throws(() => ensureFirstBootRetention(parameters))
    assert.equal(calls.length, 1, "partial identity must not trigger initialization again")
    assert.equal(existsSync(join(backupRoot, "synthetic.json")), false)
  } finally { rmSync(parent, {recursive: true}) }
})

import { verifyHomeEntryMetadata, verifyCaptureHelperVolume } from "../apps/kernel/slice-linux-docker/protected-home-capture.mjs"
test("capture helper remains bound to its owner after a restored home generation replaces the default", () => {
  const helper = "chariox-slice-synthetic-home-archive-123"
  assert.doesNotThrow(() => verifyCaptureHelperVolume(helper, "chariox-slice-synthetic-home"))
  assert.doesNotThrow(() => verifyCaptureHelperVolume(helper, `chariox-slice-synthetic-home-g${"a".repeat(32)}`))
  for (const volume of ["chariox-slice-foreign-home", "chariox-slice-synthetic-home-gbad", "chariox-slice-synthetic-home-g../private"]) {
    assert.throws(() => verifyCaptureHelperVolume(helper, volume))
  }
})
test("capture inspects filenames and link metadata before streaming any browser data", () => {
  assert.doesNotThrow(() => verifyHomeEntryMetadata(Buffer.from(".config\0d\0\0.config/chromium\0d\0\0notes\0f\0\0shortcut\0l\0notes\0")))
  assert.throws(() => verifyHomeEntryMetadata(Buffer.from(".claude/.credentials.json\0f\0\0")))
  assert.throws(() => verifyHomeEntryMetadata(Buffer.from("shortcut\0l\0/var/lib/chariox/slice-private/kernel\0")))
  assert.throws(() => verifyHomeEntryMetadata(Buffer.from("shortcut\0l\0../outside\0")))
  assert.throws(() => verifyHomeEntryMetadata(Buffer.from("fifo\0p\0\0")))
  assert.throws(() => verifyHomeEntryMetadata(Buffer.from("truncated\0f")))
})

import { requireSafeHomeVolume } from "../apps/kernel/slice-linux-docker/protected-home-preflight.mjs"
test("request preflight scans the exact volume without creating a helper or changing state", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-home-volume-metadata-test-"))
  try {
    for (const volume of ["chariox-slice-synthetic-home", `chariox-slice-synthetic-home-g${"a".repeat(32)}`]) {
    const directory = join(parent, volume)
    mkdirSync(directory, {mode: 0o700})
    const home = join(directory, "_data")
    mkdirSync(home, {mode: 0o700})
    writeFileSync(join(home, "notes"), "synthetic ordinary data", {mode: 0o600})
    const calls = []
    const docker = args => { calls.push(args); return {status: 0, stdout: JSON.stringify([{Name: volume, Mountpoint: home}])} }
    assert.doesNotThrow(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent}))
    assert.deepEqual(calls, [["volume", "inspect", volume]])
    mkdirSync(join(home, ".claude"), {mode: 0o700})
    writeFileSync(join(home, ".claude/.credentials.json"), "synthetic sentinel, not a credential", {mode: 0o600})
    assert.throws(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent}))
    assert.equal(readFileSync(join(home, "notes"), "utf8"), "synthetic ordinary data")
    assert.throws(() => requireSafeHomeVolume({volume, volumeRoot: parent,
      docker: () => ({status: 0, stdout: JSON.stringify([{Name: volume, Mountpoint: parent}])})}))
    }
  } finally { rmSync(parent, {recursive: true}) }
})

test("running browser links reach quiescence before strict capture validation", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-running-browser-metadata-test-"))
  try {
    const volume = "chariox-slice-synthetic-running-home"
    const home = join(parent, volume, "_data")
    mkdirSync(home, {recursive: true, mode: 0o700})
    const profile = join(home, ".chariox/browser/chromium")
    mkdirSync(profile, {recursive: true, mode: 0o700})
    const singleton = join(profile, "SingletonSocket")
    symlinkSync("/tmp/.org.chromium.synthetic/SingletonSocket", singleton)
    const docker = () => ({status: 0, stdout: JSON.stringify([{Name: volume, Mountpoint: home}])})
    assert.doesNotThrow(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent, quiesced: false}))
    assert.throws(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent}))
    unlinkSync(singleton) // Models the production browser stop; real Chromium proof is separate.
    assert.doesNotThrow(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent}))
    mkdirSync(join(home, ".config/gh"), {recursive: true, mode: 0o700})
    assert.throws(() => requireSafeHomeVolume({volume, docker, volumeRoot: parent, quiesced: false}))
  } finally { rmSync(parent, {recursive: true}) }
})

import { recordCapturedImageProof } from "../apps/kernel/slice-linux-docker/protected-image-proof.mjs"
test("saved image lineage requires the actual parent layers and unchanged container environment", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-saved-image-proof-test-"))
  try {
    const source = `sha256:${"b".repeat(64)}`
    const parent = {Id: digest, Config: {User: "slice"}, RootFS: {Layers: [`sha256:${"c".repeat(64)}`]}}
    recordManagedImageProof(root, source, parent)
    const container = {Image: digest, Config: {Env: ["HOME=/home/slice"]}}
    const captured = {Id: `sha256:${"d".repeat(64)}`, Parent: digest,
      Config: {User: "slice", Env: [...container.Config.Env]}, RootFS: {Layers: [...parent.RootFS.Layers, `sha256:${"e".repeat(64)}`]}}
    recordCapturedImageProof(root, source, parent, container, captured)
    assert.equal(requireManagedImageProof(root, source, captured.Id), captured.Id)
    assert.throws(() => recordCapturedImageProof(root, source, parent, container, {...captured, Parent: "unknown"}))
    assert.throws(() => recordCapturedImageProof(root, source, parent, container, {...captured, Config: {...captured.Config, Env: ["TOKEN=synthetic"]}}))
    assert.throws(() => recordCapturedImageProof(root, source, parent, container, {...captured, RootFS: {Layers: [`sha256:${"f".repeat(64)}`, `sha256:${"e".repeat(64)}`]}}))
  } finally { rmSync(root, {recursive: true}) }
})

test("capture requires a paused or stopped source and rejects every other writable home alias", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-capture-quiescence-test-"))
  try {
    const receiptRoot = join(root, "receipts")
    mkdirSync(receiptRoot, {mode: 0o700})
    const container = "chariox-slice-synthetic"
    const home = `${container}-home`
    writeProtectedLayoutReceipt(receiptRoot, container, {version: 1, sliceId: container, containerId: "synthetic-container", homeVolume: home, homeSource: "/synthetic/volumes/home/_data"})
    const info = {Id: "synthetic-container", State: {Running: true, Paused: true}}
    let otherMounts = [{Type: "volume", Name: home, RW: false}]
    const controller = createManagedLayoutController({root, sourceDigest: digest, dataOwner: process.getuid(), docker: args => {
      if (args[0] === "ps") return {status: 0, stdout: `${container}\nowned-readonly-helper\n`}
      if (args.includes("--format")) return {status: 0, stdout: JSON.stringify(otherMounts)}
      return {status: 0, stdout: JSON.stringify([info])}
    }})
    assert.doesNotThrow(() => controller.requireQuiescedHome(container))
    info.State.Paused = false
    assert.throws(() => controller.requireQuiescedHome(container))
    info.State.Running = false
    assert.doesNotThrow(() => controller.requireQuiescedHome(container))
    otherMounts[0].RW = true
    assert.throws(() => controller.requireQuiescedHome(container))
    for (const source of ["/synthetic/volumes/home/_data", "/synthetic/volumes/home/_data/subdir", "/synthetic/volumes"]) {
      otherMounts = [{Type: "bind", Source: source, RW: true}]
      assert.throws(() => controller.requireQuiescedHome(container))
    }
    otherMounts = []
    info.Id = "foreign-container"
    assert.throws(() => controller.requireQuiescedHome(container))
  } finally { rmSync(root, {recursive: true}) }
})

test("retained identity discovery skips the kernel active registry directory", async () => {
  const {retainedIdentityPaths} = await import("../apps/kernel/slice-linux-docker/protected-managed-layout.mjs")
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-identity-paths-test-"))
  try {
    const kernels = join(parent, "kernel/kernels")
    mkdirSync(join(kernels, "kernel-synthetic"), {recursive: true, mode: 0o700})
    mkdirSync(join(kernels, "active"), {mode: 0o700})
    writeFileSync(join(kernels, "active/kernel-synthetic.json"), "synthetic registration", {mode: 0o600})
    writeFileSync(join(kernels, "registry.json"), "synthetic registry", {mode: 0o600})
    assert.deepEqual(retainedIdentityPaths(parent), ["kernel/kernels/kernel-synthetic/identity.json", "kernel/kernels/registry.json"])
  } finally { rmSync(parent, {recursive: true}) }
})

test("managed layout directories keep their exact modes under the broker service umask", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-layout-umask-test-"))
  const previous = process.umask(0o007) // chariox-slice-broker.service sets UMask=0007
  try {
    const root = join(parent, "durable")
    const docker = args => {
      if (args[0] === "ps" || args[0] === "volume") return {status: 0, stdout: ""}
      throw new Error("unexpected synthetic Docker operation")
    }
    const controller = createManagedLayoutController({root, sourceDigest: `sha256:${"b".repeat(64)}`, docker, dataOwner: process.getuid()})
    const privateRoot = controller.prepare("provision", {CHARIOX_SLICE_NAME: "chariox-slice-synthetic", CHARIOX_SLICE_ID: "slice-synthetic"})
    assert.equal(existsSync(privateRoot), true)
    assert.equal(lstatSync(root).mode & 0o777, 0o711)
    assert.equal(lstatSync(join(root, "homes")).mode & 0o777, 0o711)
  } finally {
    process.umask(previous)
    rmSync(parent, {recursive: true})
  }
})



test("release F-created slice saves and backs up after upgrade with explicit legacy inventory; protected layouts still refuse", async () => {
  const {captureProtectedHome} = await import("../apps/kernel/slice-linux-docker/protected-home-capture.mjs")
  const {capturePrivateHomeArchive} = await import("../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs")
  const {spawnSync} = await import("node:child_process")
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-release-f-legacy-"))
  try {
    const root = join(parent, "durable"), home = join(parent, "synthetic-home")
    mkdirSync(home, {mode: 0o700})
    writeFileSync(join(home, "release-f-user-file"), "synthetic release F retained home", {mode: 0o600})
    const container = "chariox-slice-release-f"
    const image = {Id: digest, Config: {Labels: {"io.chariox.relay-peer-protocol-version": "68"}}}
    let info = {Id: "release-f-created-container", Image: digest, State: {Running: false},
      Mounts: [{Type: "volume", Name: `${container}-home`, Source: "/synthetic/volume", Destination: "/home/slice", RW: true}],
      Config: {Env: ["HOME=/home/slice"]}}
    const docker = args => {
      if (args[0] === "ps") return {status: 0, stdout: `${container}\n`}
      return {status: 0, stdout: JSON.stringify([args[0] === "image" ? image : info])}
    }
    // This is the upgraded controller adopting the pre-Apps Docker inspection.
    const controller = createManagedLayoutController({root, sourceDigest: digest, docker, dataOwner: process.getuid()})
    const layout = controller.captureLayout(container)
    assert.equal(layout.layoutKind, "legacy-release-f")
    assert.equal(readProtectedLayoutReceipt(join(root, "legacy-layouts"), container).containerId, info.Id)
    controller.requireQuiescedHome(container)
    const helper = `${container}-home-archive-1`
    const helperDocker = () => ({status: 0, stdout: JSON.stringify([{
      Config: {Labels: {"io.chariox.snapshot-helper": helper}}, HostConfig: {NetworkMode: "none"},
      Mounts: [{Type: "volume", Name: layout.homeVolume, Destination: "/home-src", RW: false}]}])})
    const fs = await import("node:fs"), paths = await import("node:path")
    const {runInNewContext} = await import("node:vm")
    const brokerSource = fs.readFileSync(new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url), "utf8")
    const captureSource = brokerSource.slice(brokerSource.indexOf("function validateArtifactIdentity("),
      brokerSource.indexOf("function managedHomeArchiveCoordinates("))
      .replaceAll("dirname(fileURLToPath(import.meta.url))", '"/synthetic/runtime"')
    const capture = runInNewContext(`${captureSource}\ncaptureHomeArchive`, {
      ...fs, ...paths, process, Buffer, protectedLayouts: controller, BROKER_ARTIFACT_ROOT: join(parent, "artifacts"),
      exactKeys: (value, keys) => assert.equal(Object.keys(value).sort().join(), [...keys].sort().join()),
      fail: message => {throw new Error(message)}, dockerEnvironment: () => ({PATH: "/usr/bin:/bin"}),
      spawnBounded: async (_command, args, options) => {
        assert.equal(args.at(-1), "legacy-release-f", "the broker selects release F capture only after admission")
        assert.equal(options.timeout, undefined, "legacy capture retains its progress policy")
        const result = await captureProtectedHome({helper: args[1], volume: args[2], path: args[3],
          docker: helperDocker, environment: options.env, legacy: true,
          legacyCapture: streamOptions => capturePrivateHomeArchive({...streamOptions, command: "/usr/bin/tar",
            args: ["--zstd", "-C", home, "-cf", "-", "."], minimumFreeBytes: 0})})
        return {status: 0, stdout: Buffer.from(JSON.stringify(result))}
      },
    })
    for (const scope of ["state", "backup"]) {
      const result = await capture({container: helper, scope, id: `${container}-${scope}`})
      assert.ok(result.sizeBytes > 0)
      assert.match(result.sha256, /^[a-f0-9]{64}$/)
      assert.equal(spawnSync("/usr/bin/tar", ["--zstd", "-xOf", result.path, "./release-f-user-file"], {encoding: "utf8"}).stdout,
        "synthetic release F retained home")
    }
    await assert.rejects(capture({container: helper, scope: "backup", id: `${container}-backup`}), /already exists/)
    info = {...info, Id: "replaced-container", Mounts: [{...info.Mounts[0], Source: "/different/home"}]}
    assert.throws(() => controller.captureLayout(container), "a retained legacy decision cannot admit a different home")
    info.Mounts[0].Source = "/synthetic/volume"
    info = {...info, Mounts: [...info.Mounts, {Destination: PRIVATE_ROOT}]}
    assert.throws(() => controller.captureLayout(container), "an unverified protected layout cannot become legacy")
    info = {...info, Mounts: info.Mounts.slice(0, 1)}
    image.Config.Labels["io.chariox.relay-peer-protocol-version"] = "69"
    assert.throws(() => controller.captureLayout(container), "new slices remain protected-only")
  } finally { rmSync(parent, {recursive: true, force: true}) }
})


test("legacy upgrade survives worker recreation, repeated provision, save and restore without granting protected fallback", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-legacy-recreate-"))
  try {
    const root = join(parent, "durable"), container = "chariox-slice-upgraded-legacy"
    const source = `sha256:${"d".repeat(64)}`
    const old = {Id: digest, Config: {User: "slice", Labels: {"io.chariox.relay-peer-protocol-version": "68"}}, RootFS: {Layers: [digest]}}
    const current = {Id: `sha256:${"b".repeat(64)}`, Config: {User: "slice", Labels: {"io.chariox.relay-peer-protocol-version": "69"}}, RootFS: {Layers: [digest]}}
    const images = new Map([[old.Id, old], [current.Id, current]])
    let info = {Id: "release-f-container", Image: old.Id, Config: {Env: ["HOME=/home/slice"]},
      Mounts: [{Type: "volume", Name: `${container}-home`, Source: "/synthetic/home", Destination: "/home/slice", RW: true}]}
    const docker = args => args[0] === "ps" ? {status: 0, stdout: `${container}\n`}
      : {status: 0, stdout: JSON.stringify([args[0] === "image" ? images.get(args[2]) : info])}
    const controller = createManagedLayoutController({root, sourceDigest: source, docker, dataOwner: process.getuid()})
    const env = {CHARIOX_SLICE_NAME: container, CHARIOX_SLICE_ID: "slice-upgraded-legacy"}
    assert.equal(controller.prepare("provision", env), null)
    recordManagedImageProof(controller.imageRoot, source, current)
    info = {...info, Id: "recreated-container", Image: current.Id}
    controller.complete(env)
    assert.equal(controller.prepare("provision", env), null)
    assert.equal(controller.captureLayout(container).layoutKind, "legacy-release-f")
    const saved = {Id: `sha256:${"c".repeat(64)}`, Parent: current.Id, Config: {...current.Config, Env: info.Config.Env}, RootFS: {Layers: [digest, `sha256:${"e".repeat(64)}`]}}
    images.set(saved.Id, saved)
    controller.recordLegacyImage(container, current, info, saved)
    assert.equal(controller.prepare("restore-state", {...env, CHARIOX_SLICE_SAVED_HOME_ARCHIVE: "/synthetic/owned-archive", CHARIOX_SLICE_DOCKER_IMAGE: saved.Id}), null)
    info = {...info, Id: "restored-container", Image: saved.Id}
    controller.complete(env)
    assert.equal(controller.captureLayout(container).imageId, saved.Id)
    assert.equal(controller.prepare("provision", env), null)
    info.Mounts.push({Destination: PRIVATE_ROOT})
    assert.throws(() => controller.captureLayout(container))
    assert.throws(() => controller.prepare("provision", env))
  } finally { rmSync(parent, {recursive: true, force: true}) }
})
