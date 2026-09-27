import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, verify } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, open, readFile, readdir, realpath, rename, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawn, spawnSync } from "node:child_process"
import { test } from "node:test"
import { runManagedKernelReleaseBuild } from "./build-managed-kernel-release.mjs"
import { acquireManagedReleaseBuilderLease } from "./managed-release-settlement.mjs"
import { parseBuildHistoryList } from "./managed-release-settlement.mjs"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const builderScript = join(repositoryRoot, "scripts/build-managed-kernel-release.mjs")
const sourceCommit = "a".repeat(40)
const sourceTree = "b".repeat(40)
const sourceBlob = "c".repeat(40)
const builderName = "chariox-path1-release-20260926-2cpu8g"
const buildkitNode = "chariox-path1-release-20260926-2cpu8g0"
const buildkitEndpoint = "unix:///Users/miguel/.chariox/dev/browser-computer-use/path1-build-20260926.sock"
const historyBuildId = "blix8xemjjxks864hv4fmtwj0"
const memoryGiB = 1024 ** 3

function stateDirectory(home) {
  return join(home, ".local", "state", "chariox", "managed-kernel-release")
}

function builderBarrierPath(home) {
  const key = createHash("sha256").update(builderName).digest("hex")
  return join(stateDirectory(home), `builder-${key}.unresolved.json`)
}

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
  pidsLimit = 1024,
  omitPidsLimit = false,
} = {}) {
  const hostConfig = {
    NanoCpus: nanoCpus,
    CpuQuota: cpuQuota,
    CpuPeriod: cpuPeriod,
    CpusetCpus: "",
    Memory: memory,
    MemorySwap: memorySwap,
  }
  if (!omitPidsLimit) hostConfig.PidsLimit = pidsLimit
  return JSON.stringify({
    Id: id,
    Name: `/${name}`,
    State: { Running: true, StartedAt: "2026-09-27T10:00:00Z" },
    HostConfig: hostConfig,
  })
}

async function makeFixture(root) {
  root = await realpath(root)
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
  if [ "$inspect_count" -eq 2 ] && { [ "$scenario" = post-build-identity-change ] || [ "$scenario" = post-build-pid-limit-change ]; }; then
    cat "$bin_dir/container-inspect-second.json"
  else cat "$bin_dir/container-inspect.json"; fi
  exit 0
fi
if [ "$1" = buildx ] && [ "$2" = history ] && [ "$3" = ls ]; then
  [ "$4" = --builder ] && [ "$5" = '${builderName}' ] || exit 101
  [ "$6" = --format ] && [ "$7" = json ] && [ "$8" = --no-trunc ] || exit 102
  if [ ! -f "$bin_dir/build-started" ]; then exit 0; fi
  case "$scenario" in
    timeout-unreadable) exit 103 ;;
    timeout-terminal|interrupt-terminal|restart-resolved|timeout-inspect-missing-context|timeout-inspect-missing-target)
      build_ref=$(cat "$bin_dir/build-ref")
      created_at=$(cat "$bin_dir/build-started-at")
      completed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      printf '{"ref":"%s","name":"source/apps/kernel/slice-linux-docker/docker (rust-builder)","status":"Completed","created_at":"%s","completed_at":"%s","total_steps":24,"completed_steps":24,"cached_steps":0}\\n' "$build_ref" "$created_at" "$completed_at"
      ;;
    timeout-wrong-reference)
      created_at=$(cat "$bin_dir/build-started-at")
      completed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      printf '{"ref":"${builderName}/${buildkitNode}/wrongreferenceid0123456789ab","name":"source/apps/kernel/slice-linux-docker/docker (rust-builder)","status":"Completed","created_at":"%s","completed_at":"%s","total_steps":24,"completed_steps":24,"cached_steps":0}\\n' "$created_at" "$completed_at"
      ;;
    *) exit 0 ;;
  esac
  exit 0
fi
if [ "$1" = buildx ] && [ "$2" = history ] && [ "$3" = inspect ]; then
  [ "$4" = --builder ] && [ "$5" = '${builderName}' ] || exit 104
  [ "$6" = --format ] && [ "$7" = json ] || exit 105
  reference=$8
  build_ref=$(cat "$bin_dir/build-ref")
  started_at=$(cat "$bin_dir/build-started-at")
  printf '%s\\n' "$PWD" >> "$bin_dir/history-inspect-cwds"
  history_ref=\${build_ref##*/}
  case "$scenario" in
    timeout-wrong-reference) history_ref='different-reference-id' ;;
  esac
  case "$scenario" in
    timeout-metadata-running) status=running; completed_at='' ;;
    *) status=completed; completed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ) ;;
  esac
  context_field='"Context":".",'
  target_field='"Target":"managed-release-artifacts",'
  completion_field=
  if [ -n "$completed_at" ]; then completion_field='"CompletedAt":"'"$completed_at"'",'; fi
  case "$scenario" in
    timeout-inspect-missing-context) context_field= ;;
    timeout-inspect-missing-target) target_field= ;;
  esac
  printf '{"Name":"source/apps/kernel/slice-linux-docker/docker (rust-builder)","Ref":"%s",%s%s"StartedAt":"%s",%s"Duration":100,"Status":"%s","KeepGitDir":false,"NumCompletedSteps":24,"NumTotalSteps":24,"NumCachedSteps":0,"Config":{}}\\n' "$history_ref" "$context_field" "$target_field" "$started_at" "$completion_field" "$status"
  exit 0
fi
if [ "$1" = buildx ] && [ "$2" = build ]; then
  metadata_path=
  context_path=
  previous=
  for argument do
    if [ "$previous" = --metadata-file ]; then metadata_path=$argument; fi
    context_path=$argument
    previous=$argument
  done
  build_id='${historyBuildId}'
  build_ref='${builderName}/${buildkitNode}/'"$build_id"
  printf '%s\\n' "$context_path" > "$bin_dir/build-context"
  printf '%s\\n' "$build_id" > "$bin_dir/build-id"
  printf '%s\\n' "$build_ref" > "$bin_dir/build-ref"
  date -u +%Y-%m-%dT%H:%M:%SZ > "$bin_dir/build-started-at"
  : > "$bin_dir/build-started"
  if [ "$scenario" = timeout-metadata-running ]; then
    printf '{"buildx.build.ref":"%s"}\\n' "$build_ref" > "$metadata_path"
  fi
  case "$scenario" in
    timeout-terminal|timeout-still-running|timeout-unreadable|timeout-wrong-reference|timeout-metadata-running|interrupt-terminal)
      build_pid=
      trap 'printf cancelled > "$bin_dir/build-cancelled"; [ -z "$build_pid" ] || kill "$build_pid" 2>/dev/null || true' TERM
      sleep 20 &
      build_pid=$!
      wait "$build_pid"
      ;;
    build-timeout-escaped-holder)
    '${process.execPath}' -e 'const {spawn}=require("node:child_process"); const fs=require("node:fs"); const holder=spawn(process.execPath, ["-e", "setInterval(()=>{}, 1000)"], {detached:true, stdio:["ignore",1,2]}); holder.unref(); fs.writeFileSync(process.argv[1], String(holder.pid))' "$bin_dir/escaped-child-pid"
    sleep 20
    ;;
  esac
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
  printf '{"buildx.build.ref":"%s"}\\n' "$build_ref" > "$metadata_path"
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
    root,
    bin,
    home,
    keys,
    output,
    source,
    state: stateDirectory(home),
    temp,
    trace,
    async configure({ scenario = "bounded", builderOutput, containerOutput, secondContainerOutput } = {}) {
      await writeFile(scenarioPath, scenario)
      await writeFile(inspectPath, builderOutput ?? buildxInspect())
      await writeFile(containerPath, containerOutput ?? builderContainer())
      await writeFile(join(bin, "container-inspect-second.json"), secondContainerOutput ?? builderContainer({ id: "e".repeat(64) }))
      await writeFile(join(bin, "inspect-count"), "0")
      await writeFile(trace, "")
    },
    environment() {
      return {
        ...process.env,
        PATH: `${bin}:${process.env.PATH ?? "/usr/bin:/bin"}`,
        HOME: home,
        TMPDIR: temp,
      }
    },
    options() {
      return {
        "source-repository": source,
        "source-commit": sourceCommit,
        "builder-signing-key": keyPath,
        output,
        builder: builderName,
        "preflight-timeout-seconds": 5,
        "build-timeout-seconds": 30,
      }
    },
    run(extraArguments = [], includeBuilder = true, executionTimeoutMs, cwd) {
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
        cwd,
        env: this.environment(),
      })
    },
    start(extraArguments = [], includeBuilder = true) {
      const child = spawn(process.execPath, [
        builderScript,
        "--source-repository", source,
        "--source-commit", sourceCommit,
        "--builder-signing-key", keyPath,
        ...(includeBuilder ? ["--builder", builderName] : []),
        "--output", output,
        ...extraArguments,
      ], {
        stdio: ["ignore", "ignore", "pipe"],
        env: this.environment(),
      })
      let stderr = ""
      child.stderr.setEncoding("utf8")
      child.stderr.on("data", (chunk) => { stderr += chunk })
      return { child, get stderr() { return stderr } }
    },
  }
}

async function waitForFile(path, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      await readFile(path)
      return
    } catch {}
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 25))
  }
  throw new Error(`timed out waiting for fixture marker ${path}`)
}

test("Buildx 0.30.1 history ls parses lowercase newline-delimited full references", () => {
  const firstRef = `${builderName}/${buildkitNode}/${historyBuildId}`
  const first = '{"cached_steps":0,"completed_at":"2026-09-26T19:03:38.396206506Z","completed_steps":24,"created_at":"2026-09-26T18:09:56.258036278Z","name":"source/apps/kernel/slice-linux-docker/docker (rust-builder)","ref":"' + firstRef + '","status":"Completed","total_steps":24}'
  const secondRef = `${builderName}/${buildkitNode}/blix8xemjjxks864hv4fmtwj1`
  const second = first.replace(firstRef, secondRef)
  assert.deepEqual(parseBuildHistoryList(""), [])
  assert.deepEqual(parseBuildHistoryList(first), [firstRef])
  assert.deepEqual(parseBuildHistoryList(`${first}\n${second}`), [firstRef, secondRef])
  assert.throws(() => parseBuildHistoryList('[{"ID":"blix8xemjjxks864hv4fmtwj0"}]'))
})

function waitForChild(child, timeoutMs = 10_000) {
  return new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => {
      child.kill("SIGKILL")
      reject(new Error("release fixture process did not exit"))
    }, timeoutMs)
    child.once("close", (status, signal) => {
      clearTimeout(timer)
      resolvePromise({ status, signal })
    })
  })
}

function faultingBarrierFileOps(fixture, fault, beforeRename) {
  const barrierPath = builderBarrierPath(fixture.home)
  let injected = false
  const tempPath = (path) => path.startsWith(`${barrierPath}.`) && path.endsWith(".tmp")
  const ioFailure = (operation) => Object.assign(new Error(`injected barrier ${operation} failure`), { code: "EIO" })
  return {
    async open(path, ...args) {
      if (fault === "open" && tempPath(path) && !injected) {
        injected = true
        throw ioFailure("open")
      }
      const handle = await open(path, ...args)
      const kind = tempPath(path) ? "temp" : path === fixture.state ? "directory" : null
      if (!kind) return handle
      return new Proxy(handle, {
        get(target, property) {
          if (kind === "temp" && fault === "write" && property === "writeFile" && !injected) {
            injected = true
            return async () => { throw ioFailure("write") }
          }
          if (property === "sync" &&
              ((kind === "temp" && fault === "file-sync") ||
                (kind === "directory" && fault === "directory-sync")) && !injected) {
            injected = true
            return async () => { throw ioFailure(`${kind} sync`) }
          }
          const value = Reflect.get(target, property, target)
          return typeof value === "function" ? value.bind(target) : value
        },
      })
    },
    lstat: async (path) => {
      if (fault === "foreign-barrier" && path === barrierPath && !injected) {
        const entries = await readdir(dirname(barrierPath))
        if (entries.some((entry) => entry.startsWith(`${basename(barrierPath)}.`) && entry.endsWith(".tmp"))) {
          injected = true
          await writeFile(path, "foreign barrier\n", { flag: "wx", mode: 0o600 })
        }
      }
      return lstat(path)
    },
    readFile,
    rename: async (from, to) => {
      if (to === barrierPath && tempPath(from)) {
        await beforeRename?.()
        if (fault === "rename" || fault === "tampered-pending") throw ioFailure("rename")
      }
      return rename(from, to)
    },
    rm,
  }
}

function runBuildWithBarrierFault(fixture, fault, beforeRename) {
  const fileOps = faultingBarrierFileOps(fixture, fault, beforeRename)
  return runManagedKernelReleaseBuild(fixture.options(), {
    environment: fixture.environment(),
    acquireBuilderLease: ({ home, builderName: leasedBuilderName }) =>
      acquireManagedReleaseBuilderLease({
        home,
        builderName: leasedBuilderName,
        fileOps,
      }),
  })
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
      label: "missing PID limit",
      container: builderContainer({ omitPidsLimit: true }),
      message: /explicit finite positive PID limit/,
    },
    {
      label: "unlimited PID limit",
      container: builderContainer({ pidsLimit: null }),
      message: /explicit finite positive PID limit/,
    },
    {
      label: "zero PID limit",
      container: builderContainer({ pidsLimit: 0 }),
      message: /explicit finite positive PID limit/,
    },
    {
      label: "negative PID limit",
      container: builderContainer({ pidsLimit: -1 }),
      message: /explicit finite positive PID limit/,
    },
    {
      label: "malformed PID limit",
      container: builderContainer({ pidsLimit: "1024" }),
      message: /explicit finite positive PID limit/,
    },
    {
      label: "PID limit above the release-build envelope",
      container: builderContainer({ pidsLimit: 1025 }),
      message: /PID cap exceeds the release-build limit/,
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
  assert.equal(trace.length, 7)
  assert.equal(trace[0], `buildx inspect ${builderName}`)
  assert.ok(trace[1].startsWith(`--host ${buildkitEndpoint} inspect --type container --format `), trace[1])
  assert.equal(trace[2], `buildx history ls --builder ${builderName} --format json --no-trunc`)
  assert.ok(trace[3].startsWith(`buildx build --builder ${builderName} --pull --platform linux/amd64 --target managed-release-artifacts --metadata-file `), trace[3])
  assert.ok(trace[4].startsWith(`buildx history inspect --builder ${builderName} --format json `), trace[4])
  assert.equal(trace[4].slice(`buildx history inspect --builder ${builderName} --format json `.length), historyBuildId)
  assert.equal(trace[5], `buildx inspect ${builderName}`)
  assert.ok(trace[6].startsWith(`--host ${buildkitEndpoint} inspect --type container --format `), trace[6])
  assert.doesNotMatch(trace.join("\n"), /--bootstrap|buildx create|buildx stop|buildx rm/)
  assert.deepEqual((await readdir(fixture.output)).sort(), [
    "build-attestation.json", "build-attestation.sig", "builder-public-key",
    "chariox-kernel", "chariox-managed-bootstrap", "chariox-relay",
  ])
  const attestation = await readFile(join(fixture.output, "build-attestation.json"))
  const signature = Buffer.from(await readFile(join(fixture.output, "build-attestation.sig"), "utf8"), "base64")
  assert.equal(verify(null, attestation, fixture.keys.publicKey, signature), true)
})

test("initial barrier write failures clean only owned scratch and never build or sign", async (context) => {
  for (const fault of ["open", "write", "file-sync", "rename"]) {
    const root = await mkdtemp(join(tmpdir(), `chariox-managed-build-barrier-${fault}-`))
    context.after(() => rm(root, { recursive: true, force: true }))
    const fixture = await makeFixture(root)
    await fixture.configure({ scenario: "bounded" })

    await assert.rejects(
      runBuildWithBarrierFault(fixture, fault),
      (error) => error.barrierDisposition === "absent",
      fault,
    )
    assert.equal(await readFile(builderBarrierPath(fixture.home)).then(() => true, () => false), false, fault)
    assert.equal(await lstat(fixture.output).then(() => true, () => false), false, fault)
    assert.deepEqual(await readdir(fixture.state), [], fault)
    assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [], fault)
    assert.deepEqual(await readdir(fixture.temp), [], fault)
    assert.doesNotMatch(await readFile(fixture.trace, "utf8"), /buildx build/, fault)
    assert.equal(await readFile(join(fixture.output, "build-attestation.sig")).then(() => true, () => false), false, fault)
  }
})

test("a post-rename directory-sync failure retains a prepared barrier for safe recovery", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-barrier-directory-sync-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await fixture.configure({ scenario: "bounded" })

  await assert.rejects(
    runBuildWithBarrierFault(fixture, "directory-sync"),
    (error) => error.barrierDisposition === "persisted",
  )
  const barrierPath = builderBarrierPath(fixture.home)
  const barrier = JSON.parse(await readFile(barrierPath, "utf8"))
  assert.equal(barrier.buildStarted, false)
  assert.ok(await readFile(join(barrier.sourceDirectory, "apps/kernel/slice-linux-docker/docker/Dockerfile")))
  assert.deepEqual(await readdir(barrier.pendingDirectory), [])
  assert.doesNotMatch(await readFile(fixture.trace, "utf8"), /buildx build/)
  assert.equal(await readFile(join(fixture.output, "build-attestation.sig")).then(() => true, () => false), false)

  const recovered = await runManagedKernelReleaseBuild(fixture.options(), {
    environment: fixture.environment(),
  })
  assert.equal(recovered, undefined)
  assert.equal(await readFile(barrierPath).then(() => true, () => false), false)
  assert.equal(await lstat(barrier.sourceDirectory).then(() => true, () => false), false)
  assert.deepEqual((await readdir(fixture.output)).sort(), [
    "build-attestation.json", "build-attestation.sig", "builder-public-key",
    "chariox-kernel", "chariox-managed-bootstrap", "chariox-relay",
  ])
  assert.match(await readFile(join(fixture.output, "build-attestation.sig"), "utf8"), /^[A-Za-z0-9+/]+=*\n?$/)
  const trace = await readFile(fixture.trace, "utf8")
  assert.equal((trace.match(/buildx build --builder/g) ?? []).length, 1)
})

test("foreign barrier and replaced scratch paths fail closed", async (context) => {
  const foreignRoot = await mkdtemp(join(tmpdir(), "chariox-managed-build-foreign-barrier-"))
  context.after(() => rm(foreignRoot, { recursive: true, force: true }))
  const foreignFixture = await makeFixture(foreignRoot)
  await foreignFixture.configure({ scenario: "bounded" })
  await assert.rejects(
    runBuildWithBarrierFault(foreignFixture, "foreign-barrier"),
    (error) => error.barrierDisposition === "unknown",
  )
  assert.equal(await readFile(builderBarrierPath(foreignFixture.home), "utf8"), "foreign barrier\n")
  assert.doesNotMatch(await readFile(foreignFixture.trace, "utf8"), /buildx build/)
  assert.equal((await readdir(foreignFixture.state)).some((entry) => entry.startsWith("run-")), true)
  assert.equal((await readdir(foreignRoot)).some((name) => name.startsWith(".new-output-")), true)

  const tamperedRoot = await mkdtemp(join(tmpdir(), "chariox-managed-build-tampered-scratch-"))
  context.after(() => rm(tamperedRoot, { recursive: true, force: true }))
  const tamperedFixture = await makeFixture(tamperedRoot)
  await tamperedFixture.configure({ scenario: "bounded" })
  let pendingPath
  await assert.rejects(
    runBuildWithBarrierFault(tamperedFixture, "tampered-pending", async () => {
      pendingPath = (await readdir(tamperedRoot)).map((name) => join(tamperedRoot, name))
        .find((path) => basename(path).startsWith(".new-output-"))
      await rename(pendingPath, `${pendingPath}.foreign`)
      await mkdir(pendingPath)
      await writeFile(join(pendingPath, "foreign-sentinel"), "keep\n")
    }),
    /scratch path changed; refusing cleanup/,
  )
  assert.equal(await readFile(join(pendingPath, "foreign-sentinel"), "utf8"), "keep\n")
  assert.equal(await readdir(tamperedFixture.state).then((entries) => entries.some((entry) => entry.startsWith("run-"))), true)
  assert.doesNotMatch(await readFile(tamperedFixture.trace, "utf8"), /buildx build/)
})

test("managed release build timeout waits for the exact terminal history record before cleanup", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-timeout-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await fixture.configure({ scenario: "timeout-terminal" })

  const result = fixture.run(["--build-timeout-seconds", "1"])
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /managed release build timed out after remote settlement/)
  const trace = await readFile(fixture.trace, "utf8")
  assert.ok(trace.split("\n").some((line) => line.startsWith(`buildx build --builder ${builderName} `)))
  assert.ok(trace.split("\n").some((line) => line.startsWith(`buildx history inspect --builder ${builderName} --format json `)))
  assert.equal(await readFile(join(fixture.bin, "build-cancelled"), "utf8"), "cancelled")
  assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
  assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [])
  assert.deepEqual(await readdir(fixture.temp), [])
  assert.equal(await readFile(builderBarrierPath(fixture.home)).then(() => true, () => false), false)
  assert.deepEqual(await readdir(fixture.state), [])
})

test("managed release build preserves its durable barrier when history is absent, unreadable, running, or mismatched", async (context) => {
  for (const scenario of [
    "timeout-still-running",
    "timeout-unreadable",
    "timeout-metadata-running",
    "timeout-wrong-reference",
    "timeout-inspect-missing-context",
    "timeout-inspect-missing-target",
  ]) {
    const root = await mkdtemp(join(tmpdir(), `chariox-managed-build-${scenario}-`))
    context.after(() => rm(root, { recursive: true, force: true }))
    const fixture = await makeFixture(root)
    await fixture.configure({ scenario })

    const result = fixture.run(["--build-timeout-seconds", "1"])
    assert.equal(result.status, 1, `${scenario}: ${result.stderr}`)
    assert.match(result.stderr, /managed release build settlement is unresolved/, scenario)
    const barrier = JSON.parse(await readFile(builderBarrierPath(fixture.home), "utf8"))
    assert.equal(barrier.builderName, builderName)
    assert.equal(barrier.sourceCommit, sourceCommit)
    assert.equal(barrier.builderFingerprint.nodes[0].containerId, "d".repeat(64))
    assert.ok(await readFile(join(barrier.sourceDirectory, "apps/kernel/slice-linux-docker/docker/Dockerfile")))
    assert.deepEqual(await readdir(barrier.pendingDirectory), [])
    assert.equal(await readFile(fixture.output).then(() => true, () => false), false)
    assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [basename(barrier.pendingDirectory)])
  }
})

test("managed release interruption waits for exact terminal history and does not publish", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-interruption-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await fixture.configure({ scenario: "interrupt-terminal" })

  const run = fixture.start(["--build-timeout-seconds", "30"])
  await waitForFile(join(fixture.bin, "build-started"))
  const exited = waitForChild(run.child)
  run.child.kill("SIGINT")
  const result = await exited
  assert.equal(result.status, 1, run.stderr)
  assert.match(run.stderr, /interrupted by SIGINT/)
  assert.equal(await readFile(join(fixture.bin, "build-cancelled"), "utf8"), "cancelled")
  assert.equal(await readFile(builderBarrierPath(fixture.home)).then(() => true, () => false), false)
  assert.equal(await readFile(fixture.output).then(() => true, () => false), false)
  assert.deepEqual(await readdir(root), ["bin", "builder-key.pem", "docker-home", "docker-trace", "source-repository", "tmp"])
})

test("managed release restart keeps the builder leased until the exact prior source and builder history settles", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-restart-barrier-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const callerOne = join(fixture.root, "caller-one")
  const callerTwo = join(fixture.root, "caller-two")
  await Promise.all([mkdir(callerOne), mkdir(callerTwo)])
  await fixture.configure({ scenario: "timeout-still-running" })

  const first = fixture.run(["--build-timeout-seconds", "1"], true, undefined, callerOne)
  assert.equal(first.status, 1, first.stderr)
  const barrierPath = builderBarrierPath(fixture.home)
  const barrier = JSON.parse(await readFile(barrierPath, "utf8"))
  const originalSourceFile = join(barrier.sourceDirectory, "apps/kernel/slice-linux-docker/docker/Dockerfile")
  const originalSource = await readFile(originalSourceFile)

  await fixture.configure({ scenario: "restart-resolved" })
  await writeFile(originalSourceFile, "tampered source\n")
  const changedSource = fixture.run([], true, undefined, callerTwo)
  assert.equal(changedSource.status, 1, changedSource.stderr)
  assert.match(changedSource.stderr, /retained source does not match its invocation digest/)
  assert.equal(await readFile(barrierPath).then(() => true, () => false), true)
  assert.doesNotMatch(await readFile(fixture.trace, "utf8"), /buildx build/)

  await writeFile(originalSourceFile, originalSource)
  await fixture.configure({ scenario: "restart-resolved", containerOutput: builderContainer({ id: "f".repeat(64) }) })
  const changedBuilder = fixture.run([], true, undefined, callerTwo)
  assert.equal(changedBuilder.status, 1, changedBuilder.stderr)
  assert.match(changedBuilder.stderr, /unresolved prior build: builder fingerprint changed/)
  assert.equal(await readFile(barrierPath).then(() => true, () => false), true)
  assert.doesNotMatch(await readFile(fixture.trace, "utf8"), /buildx build/)

  await fixture.configure({ scenario: "restart-resolved" })
  const recovered = fixture.run([], true, undefined, callerTwo)
  assert.equal(recovered.status, 0, recovered.stderr)
  const trace = (await readFile(fixture.trace, "utf8")).trim().split("\n")
  const oldHistoryInspect = trace.findIndex((entry) => entry.startsWith(`buildx history inspect --builder ${builderName}`))
  const nextBuild = trace.findIndex((entry) => entry.startsWith(`buildx build --builder ${builderName}`))
  assert.ok(oldHistoryInspect >= 0 && oldHistoryInspect < nextBuild, trace.join("\n"))
  const inspectedDirectories = (await readFile(join(fixture.bin, "history-inspect-cwds"), "utf8")).trim().split("\n")
  assert.equal(inspectedDirectories[0], barrier.sourceDirectory)
  assert.notEqual(inspectedDirectories[0], callerTwo)
  assert.equal(await readFile(barrierPath).then(() => true, () => false), false)
  assert.equal(await readFile(originalSourceFile).then(() => true, () => false), false)
  assert.deepEqual((await readdir(fixture.output)).sort(), [
    "build-attestation.json", "build-attestation.sig", "builder-public-key",
    "chariox-kernel", "chariox-managed-bootstrap", "chariox-relay",
  ])
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
  assert.match(result.stderr, /managed release build settlement is unresolved/)
  assert.ok(elapsedMs < 19_000, `script remained open for ${elapsedMs}ms`)
  assert.ok(Number(await readFile(join(fixture.bin, "escaped-child-pid"), "utf8")) > 0)
  assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
  assert.equal((await readdir(root)).filter((name) => name.startsWith(".new-output-")).length, 1)
  const barrier = JSON.parse(await readFile(builderBarrierPath(fixture.home), "utf8"))
  assert.ok(await readFile(join(barrier.sourceDirectory, "apps/kernel/slice-linux-docker/docker/Dockerfile")))
  assert.equal((await readdir(barrier.pendingDirectory)).length, 0)
  assert.deepEqual(await readdir(fixture.temp), [])
})

test("managed release build withholds signing if its inspected builder changes during build", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-builder-change-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  for (const changed of [
    { scenario: "post-build-identity-change", secondContainerOutput: builderContainer({ id: "e".repeat(64) }) },
    { scenario: "post-build-pid-limit-change", secondContainerOutput: builderContainer({ pidsLimit: 512 }) },
  ]) {
    await fixture.configure(changed)
    const result = fixture.run()
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /identity or resource limits changed during the release build/)
    const trace = (await readFile(fixture.trace, "utf8")).trim().split("\n")
    assert.ok(trace[3].startsWith(`buildx build --builder ${builderName}`))
    assert.equal(await readdir(root).then((entries) => entries.includes("output")), false)
    assert.deepEqual((await readdir(root)).filter((name) => name.startsWith(".new-output-")), [])
    assert.deepEqual(await readdir(fixture.temp), [])
  }
})
