import assert from "node:assert/strict"
import { generateKeyPairSync, verify } from "node:crypto"
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const builderScript = join(repositoryRoot, "scripts/build-managed-kernel-release.mjs")
const sourceCommit = "a".repeat(40)
const sourceTree = "b".repeat(40)
const sourceBlob = "c".repeat(40)
const builderName = "chariox-path1-release-20260926-2cpu8g"
const buildkitNode = "chariox-path1-release-20260926-2cpu8g0"
const buildkitEndpoint = "unix:///Users/miguel/.chariox/dev/browser-computer-use/path1-build-20260926.sock"
const memoryGiB = 1024 ** 3

function buildxInspect(driver = "docker-container", nodeName = buildkitNode, includeStatus = true) {
  return [
    `Name:          ${builderName}`,
    `Driver:        ${driver}`,
    "",
    "Nodes:",
    `Name:          ${nodeName}`,
    `Endpoint:      ${buildkitEndpoint}`,
    ...(includeStatus ? ["Status:        running"] : []),
    "",
  ].join("\n")
}

function builderContainer({
  id = "d".repeat(64),
  name = `buildx_buildkit_${buildkitNode}`,
  nanoCpus = 0,
  cpuQuota = 200_000,
  cpuPeriod = 100_000,
  memory = 8 * memoryGiB,
  memorySwap = 8 * memoryGiB,
} = {}) {
  return JSON.stringify({
    Id: id,
    Name: `/${name}`,
    State: { Running: true, StartedAt: "2026-09-27T10:00:00Z" },
    HostConfig: {
      NanoCpus: nanoCpus,
      CpuQuota: cpuQuota,
      CpuPeriod: cpuPeriod,
      CpusetCpus: "",
      Memory: memory,
      MemorySwap: memorySwap,
      PidsLimit: null,
    },
  })
}

async function makeFixture(root) {
  const bin = join(root, "bin")
  const source = join(root, "source-repository")
  const temp = join(root, "tmp")
  const home = join(root, "docker-home")
  const output = join(root, "output")
  const keyPath = join(root, "builder-key.pem")
  const trace = join(root, "docker-trace")
  await Promise.all([mkdir(bin), mkdir(source), mkdir(temp), mkdir(home)])

  const keys = generateKeyPairSync("ed25519", {
    privateKeyEncoding: { format: "pem", type: "pkcs8" },
    publicKeyEncoding: { format: "pem", type: "spki" },
  })
  await writeFile(keyPath, keys.privateKey, { mode: 0o600 })
  await chmod(keyPath, 0o600)

  const dockerfilePath = join(bin, "source.Dockerfile")
  await writeFile(dockerfilePath, "FROM scratch AS managed-release-artifacts\n")
  const fakeGit = `#!/bin/sh
set -eu
case "$1" in
  rev-parse)
    if [ "$2" = "--verify" ]; then printf '%s\\n' '${sourceCommit}'; else printf '%s\\n' '${sourceTree}'; fi
    ;;
  ls-tree)
    printf '100644 blob ${sourceBlob}\\tapps/kernel/slice-linux-docker/docker/Dockerfile\\0'
    ;;
  cat-file)
    [ "$2" = blob ] && [ "$3" = '${sourceBlob}' ] || exit 91
    cat '${dockerfilePath}'
    ;;
  *) exit 92 ;;
esac
`
  await writeFile(join(bin, "git"), fakeGit, { mode: 0o755 })

  const scenarioPath = join(bin, "scenario")
  const inspectPath = join(bin, "builder-inspect")
  const containerPath = join(bin, "container-inspect.json")
  const fakeDocker = `#!/bin/sh
set -eu
bin_dir=$(dirname "$0")
scenario=$(cat "$bin_dir/scenario")
printf '%s\\n' "$*" >> '${trace}'
if [ "$1" = buildx ] && [ "$2" = inspect ]; then
  if [ "$scenario" = inspect-timeout ]; then exec sleep 20; fi
  cat "$bin_dir/builder-inspect"
  exit 0
fi
if [ "$1" = --context ] || [ "$1" = --host ]; then
  [ "$1" = --host ] && [ "$2" = '${buildkitEndpoint}' ] || exit 93
  shift 2
fi
if [ "$1" = inspect ]; then
  [ "$2" = --type ] && [ "$3" = container ] || exit 94
  [ "$4" = --format ] && [ "$5" = '{{json .}}' ] || exit 95
  [ "$6" = 'buildx_buildkit_${buildkitNode}' ] || exit 99
  if [ "$scenario" = malformed-container ]; then printf '%s\\n' '{bad json'; exit 0; fi
  inspect_count=0
  [ ! -f "$bin_dir/inspect-count" ] || inspect_count=$(cat "$bin_dir/inspect-count")
  inspect_count=$((inspect_count + 1))
  printf '%s\\n' "$inspect_count" > "$bin_dir/inspect-count"
  if [ "$scenario" = post-build-identity-change ] && [ "$inspect_count" -eq 2 ]; then
    cat "$bin_dir/container-inspect-second.json"
  else cat "$bin_dir/container-inspect.json"; fi
  exit 0
fi
if [ "$1" = buildx ] && [ "$2" = build ]; then
  if [ "$scenario" = build-timeout ]; then
    build_pid=
    trap 'printf cancelled > "$bin_dir/build-cancelled"; [ -z "$build_pid" ] || kill "$build_pid" 2>/dev/null || true' TERM
    sleep 20 &
    build_pid=$!
    wait "$build_pid"
  elif [ "$scenario" = build-timeout-escaped-holder ]; then
    '${process.execPath}' -e 'const {spawn}=require("node:child_process"); const fs=require("node:fs"); const holder=spawn(process.execPath, ["-e", "setInterval(()=>{}, 1000)"], {detached:true, stdio:["ignore",1,2]}); holder.unref(); fs.writeFileSync(process.argv[1], String(holder.pid))' "$bin_dir/escaped-child-pid"
    sleep 20
  fi
  destination=
  previous=
  for argument do
    if [ "$previous" = --output ]; then
      case "$argument" in type=local,dest=*) destination=\${argument#type=local,dest=} ;; *) exit 96 ;; esac
    fi
    previous=$argument
  done
  [ -n "$destination" ] || exit 97
  mkdir "$destination"
  printf 'kernel\\n' > "$destination/chariox-kernel"
  printf 'bootstrap\\n' > "$destination/chariox-managed-bootstrap"
  printf 'relay\\n' > "$destination/chariox-relay"
  exit 0
fi
exit 98
`
  await writeFile(join(bin, "docker"), fakeDocker, { mode: 0o755 })
  await writeFile(trace, "")
  await writeFile(scenarioPath, "bounded")
  await writeFile(inspectPath, buildxInspect())
  await writeFile(containerPath, builderContainer())
  await writeFile(join(bin, "container-inspect-second.json"), builderContainer({ id: "e".repeat(64) }))
  await writeFile(join(bin, "inspect-count"), "0")

  return {
    bin,
    home,
    keys,
    output,
    source,
    temp,
    trace,
    async configure({ scenario = "bounded", builderOutput, containerOutput } = {}) {
      await writeFile(scenarioPath, scenario)
      await writeFile(inspectPath, builderOutput ?? buildxInspect())
      await writeFile(containerPath, containerOutput ?? builderContainer())
      await writeFile(join(bin, "inspect-count"), "0")
      await writeFile(trace, "")
    },
    run(extraArguments = [], includeBuilder = true, executionTimeoutMs) {
      return spawnSync(process.execPath, [
        builderScript,
        "--source-repository", source,
        "--source-commit", sourceCommit,
        "--builder-signing-key", keyPath,
        ...(includeBuilder ? ["--builder", builderName] : []),
        "--output", output,
        ...extraArguments,
      ], {
        encoding: "utf8",
        timeout: executionTimeoutMs,
        env: {
          ...process.env,
          PATH: `${bin}:${process.env.PATH ?? "/usr/bin:/bin"}`,
          HOME: home,
          TMPDIR: temp,
        },
      })
    },
  }
}

test("managed release build requires a bounded, already-running Buildx builder before build", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-resource-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)

  const rejected = [
    {
      label: "implicit default builder",
      scenario: "default-builder",
      includeBuilder: false,
      message: /usage:/,
    },
    {
      label: "uncapped builder",
      scenario: "uncapped",
      container: builderContainer({ nanoCpus: 0, cpuQuota: 0, cpuPeriod: 0, memory: 0, memorySwap: 0 }),
      message: /no hard CPU limit/,
    },
    {
      label: "CPU cap above the release-build envelope",
      scenario: "excessive-cpu",
      container: builderContainer({ cpuQuota: 900_000, cpuPeriod: 100_000 }),
      message: /CPU cap is outside/,
    },
    {
      label: "memory cap above the release-build envelope",
      scenario: "excessive-memory",
      container: builderContainer({ memory: 17 * memoryGiB, memorySwap: 17 * memoryGiB }),
      message: /memory cap is outside/,
    },
    {
      label: "swap cap above the release-build envelope",
      scenario: "excessive-swap",
      container: builderContainer({ memorySwap: 33 * memoryGiB }),
      message: /memory-plus-swap cap exceeds/,
    },
    {
      label: "malformed builder-container inspect",
      scenario: "malformed-container",
      message: /inspect output is malformed/,
    },
    {
      label: "wrong BuildKit container identity",
      scenario: "wrong-container",
      container: builderContainer({ name: "another-node" }),
      message: /does not match its inspected node/,
    },
    {
      label: "bare Buildx node name is not its backing container",
      scenario: "bare-node-container",
      container: builderContainer({ name: buildkitNode }),
      message: /does not match its inspected node/,
    },
    {
      label: "unbounded swap setting",
      scenario: "unbounded-swap",
      container: builderContainer({ memorySwap: -1 }),
      message: /MemorySwap is malformed/,
    },
    {
      label: "wrong driver",
      scenario: "wrong-driver",
      builder: buildxInspect("docker"),
      message: /docker-container driver/,
    },
    {
      label: "mismatched inspected builder identity",
      scenario: "wrong-builder",
      builder: buildxInspect().replace(builderName, "other-builder"),
      message: /identity does not match/,
    },
    {
      label: "inactive node",
      scenario: "inactive-node",
      builder: buildxInspect().replace("Status:        running", "Status:        inactive"),
      message: /already be running/,
    },
    {
      label: "inspection timeout",
      scenario: "inspect-timeout",
      message: /docker buildx inspect timed out/,
      extraArguments: ["--preflight-timeout-seconds", "1"],
    },
  ]

  for (const rejectedCase of rejected) {
    await fixture.configure({
      scenario: rejectedCase.scenario,
      builderOutput: rejectedCase.builder,
      containerOutput: rejectedCase.container,
    })
    const result = fixture.run(rejectedCase.extraArguments, rejectedCase.includeBuilder)
    assert.equal(result.status, 1, `${rejectedCase.label}: ${result.stderr}`)
    assert.match(result.stderr, rejectedCase.message, rejectedCase.label)
    assert.doesNotMatch(await readFile(fixture.trace, "utf8"), /buildx build/, rejectedCase.label)
    assert.equal(await readdir(root).then((entries) => entries.includes("output")), false, rejectedCase.label)
    assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [], rejectedCase.label)
    assert.deepEqual(await readdir(fixture.temp), [], rejectedCase.label)
  }

  await fixture.configure()
  const admitted = fixture.run()
  assert.equal(admitted.status, 0, admitted.stderr)
  const trace = (await readFile(fixture.trace, "utf8")).trim().split("\n")
  assert.equal(trace.length, 5)
  assert.equal(trace[0], `buildx inspect ${builderName}`)
  assert.ok(trace[1].startsWith(`--host ${buildkitEndpoint} inspect --type container --format `), trace[1])
  assert.ok(trace[2].startsWith(`buildx build --builder ${builderName} --pull --platform linux/amd64 --target managed-release-artifacts `), trace[2])
  assert.equal(trace[3], `buildx inspect ${builderName}`)
  assert.ok(trace[4].startsWith(`--host ${buildkitEndpoint} inspect --type container --format `), trace[4])
  assert.doesNotMatch(trace.join("\n"), /--bootstrap|buildx create|buildx stop|buildx rm/)
  assert.deepEqual((await readdir(fixture.output)).sort(), [
    "build-attestation.json", "build-attestation.sig", "builder-public-key",
    "chariox-kernel", "chariox-managed-bootstrap", "chariox-relay",
  ])
  const attestation = await readFile(join(fixture.output, "build-attestation.json"))
  const signature = Buffer.from(await readFile(join(fixture.output, "build-attestation.sig"), "utf8"), "base64")
  assert.equal(verify(null, attestation, fixture.keys.publicKey, signature), true)
})

test("managed release build deadline terminates a stalled build command", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-timeout-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await fixture.configure({ scenario: "build-timeout" })

  const result = fixture.run(["--build-timeout-seconds", "1"])
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /managed release build timed out/)
  const trace = await readFile(fixture.trace, "utf8")
  assert.ok(trace.split("\n").some((line) => line.startsWith(`buildx build --builder ${builderName} `)))
  assert.equal(await readFile(join(fixture.bin, "build-cancelled"), "utf8"), "cancelled")
  assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
  assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [])
  assert.deepEqual(await readdir(fixture.temp), [])
})

test("managed release build closes inherited pipes after a detached descendant survives cancellation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-escaped-output-"))
  const fixture = await makeFixture(root)
  context.after(async () => {
    try {
      const pid = Number(await readFile(join(fixture.bin, "escaped-child-pid"), "utf8"))
      if (Number.isSafeInteger(pid) && pid > 0) process.kill(pid, "SIGKILL")
    } catch {}
    await rm(root, { recursive: true, force: true })
  })
  await fixture.configure({ scenario: "build-timeout-escaped-holder" })

  const startedAt = Date.now()
  const result = fixture.run(["--build-timeout-seconds", "1"], true, 20_000)
  const elapsedMs = Date.now() - startedAt
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /managed release build timed out/)
  assert.ok(elapsedMs < 19_000, `script remained open for ${elapsedMs}ms`)
  assert.ok(Number(await readFile(join(fixture.bin, "escaped-child-pid"), "utf8")) > 0)
  assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
  assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [])
  assert.deepEqual(await readdir(fixture.temp), [])
})

test("managed release build withholds signing if its inspected builder changes during build", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-builder-change-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await fixture.configure({ scenario: "post-build-identity-change" })

  const result = fixture.run()
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /identity or resource limits changed during the release build/)
  const trace = (await readFile(fixture.trace, "utf8")).trim().split("\n")
  assert.ok(trace[2].startsWith(`buildx build --builder ${builderName}`))
  assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
  assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [])
  assert.deepEqual(await readdir(fixture.temp), [])
})
