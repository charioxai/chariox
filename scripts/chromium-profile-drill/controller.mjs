import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, realpathSync, statfsSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checked, docker, loadOwner } from "./resources.mjs";
import { repository } from "./prepare.mjs";

const sourceFiles = ["browser_controller.rs", ...[
  "actor.rs", "protocol.rs", "types.rs", "tests.rs", "tests/fixture.rs", "tests/hosted.rs",
].map(name => `browser_controller/${name}`)];
const dependencies = ["tokio", "tokio-tungstenite", "futures-util", "serde_json", "base64"];
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
function binaryDigest(path) {
  const stat = statSync(path);
  assert.ok(stat.isFile() && stat.size > 0 && stat.size <= 64 * 1024 ** 2, "unexpected controller test artifact");
  return digest(readFileSync(path));
}

export function manifestFromLock(lock) {
  const pins = {};
  for (const entry of lock.split("[[package]]").slice(1)) {
    const name = entry.match(/^name = "([^"]+)"$/m)?.[1];
    if (!dependencies.includes(name)) continue;
    const version = entry.match(/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/m)?.[1];
    assert.ok(version && !pins[name], `ambiguous or invalid locked dependency ${name}`);
    pins[name] = version;
  }
  assert.deepEqual(Object.keys(pins).sort(), [...dependencies].sort());
  const feature = name => name === "tokio" ? ", features = [\"io-util\", \"macros\", \"net\", \"rt-multi-thread\", \"sync\", \"time\"]"
    : name === "tokio-tungstenite" ? ", features = [\"rustls-tls-webpki-roots\"]" : "";
  return `[package]\nname = "chariox-browser-controller-validation"\nversion = "0.0.0"\nedition = "2021"\n[lib]\npath = "lib.rs"\n[dependencies]\n`
    + dependencies.map(name => `${name} = { version = "=${pins[name]}"${feature(name)} }\n`).join("");
}

function identity() {
  assert.equal(process.platform, "linux");
  assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
  assert.equal(process.env.GITHUB_REPOSITORY, "charioxai/chariox");
  assert.ok(process.getuid() > 0 && process.getgid() > 0, "fixture coordinator must run as the runner user");
  return { uid: process.getuid(), gid: process.getgid() };
}
function unit(name, properties, command, args, timeout) {
  return checked("sudo", ["--non-interactive", "systemd-run", "--quiet", "--wait", "--pipe", "--collect", `--unit=${name}`,
    "--property=KillMode=control-group", "--property=TimeoutStopSec=3", "--property=MemorySwapMax=0",
    ...properties.map(value => `--property=${value}`), "--", command, ...args], { timeout });
}
function treeInputs(root) {
  return Object.fromEntries(sourceFiles.map(name => {
    const bytes = readFileSync(join(root, name));
    assert.ok(bytes.length < 128 * 1024);
    return [name, digest(bytes)];
  }));
}
export function build(scratch) {
  const owner = loadOwner(scratch);
  const { uid, gid } = identity();
  const disk = statfsSync(scratch);
  assert.ok(disk.bavail * disk.bsize >= 12 * 1024 ** 3, "controller build requires 12GiB free scratch space");
  const harness = join(scratch, "controller");
  const target = join(harness, "target");
  mkdirSync(join(harness, "browser_controller/tests"), { recursive: true, mode: 0o700 });
  const source = join(repository, "apps/kernel/src/runtime");
  const inputs = treeInputs(source);
  for (const name of sourceFiles) copyFileSync(join(source, name), join(harness, name));
  assert.deepEqual(treeInputs(harness), inputs);
  const lock = readFileSync(join(repository, "Cargo.lock"), "utf8");
  writeFileSync(join(harness, "Cargo.toml"), manifestFromLock(lock));
  writeFileSync(join(harness, "Cargo.lock"), lock);
  writeFileSync(join(harness, "lib.rs"), "mod browser_controller;\n");
  // Keep the cargo executable name: resolving its rustup symlink would change
  // argv[0] and select the rustup CLI instead of the cargo proxy.
  const cargo = resolve(checked("which", ["cargo"]));
  const env = ["-i", `HOME=${process.env.HOME}`, `PATH=${process.env.PATH}`, `CARGO_TARGET_DIR=${target}`,
    "CARGO_BUILD_JOBS=1", "CARGO_INCREMENTAL=0", "CARGO_PROFILE_DEV_DEBUG=0", "CARGO_PROFILE_TEST_DEBUG=0"];
  // The copied workspace lock retains existing transitive selections. Cargo
  // adds this isolated package and removes unrelated entries; record that exact
  // resulting lock separately rather than claiming byte identity with workspace.
  const output = unit(`chariox-browser-build-${owner.id}`, [`User=${uid}`, `Group=${gid}`, "MemoryMax=1536M", "CPUQuota=100%", "TasksMax=128", "RuntimeMaxSec=180"],
    "/usr/bin/env", [...env, cargo, "test", "--manifest-path", join(harness, "Cargo.toml"), "--no-run", "--message-format=json"], 190000);
  const artifacts = output.split("\n").filter(Boolean).map(line => JSON.parse(line))
    .filter(value => value.reason === "compiler-artifact" && value.target?.name === "chariox_browser_controller_validation" && value.executable);
  assert.equal(artifacts.length, 1);
  const executable = realpathSync(artifacts[0].executable);
  assert.ok(executable.startsWith(`${realpathSync(target)}/debug/deps/`));
  assert.ok(statSync(executable).isFile());
  const log = unit(`chariox-browser-unit-${owner.id}`, [`User=${uid}`, `Group=${gid}`, "MemoryMax=256M", "CPUQuota=100%", "TasksMax=32", "RuntimeMaxSec=30"],
    "/usr/bin/env", ["-i", "PATH=/usr/bin:/bin", executable, "--test-threads=1", "--nocapture"], 40000);
  assert.match(log, /10 passed; 0 failed; 1 ignored/);
  writeFileSync(join(scratch, "evidence/controller-unit-tests.log"), log);
  const record = { revision: owner.revision, inputs, workspaceLockDigest: digest(lock),
    harnessLockDigest: digest(readFileSync(join(harness, "Cargo.lock"))),
    binaryDigest: binaryDigest(executable), executable,
    toolchain: checked("rustc", ["--version"]), unitTests: 10, hostedTestExecuted: false };
  writeFileSync(join(harness, "build.json"), JSON.stringify(record));
  writeFileSync(join(scratch, "evidence/controller-inputs.json"), JSON.stringify(record, null, 2));
}

export function runController(scratch, container) {
  const owner = loadOwner(scratch);
  const { uid, gid } = identity();
  assert.match(container, /^[a-f0-9]{64}$/);
  const inspect = JSON.parse(docker(["inspect", "--format", "{{json .}}", container]));
  assert.equal(inspect.Config.Labels["io.chariox.chromium-drill"], owner.id);
  assert.equal(inspect.State.Running, true);
  const pid = inspect.State.Pid;
  assert.ok(Number.isSafeInteger(pid) && pid > 1);
  const harness = join(scratch, "controller");
  const record = JSON.parse(readFileSync(join(harness, "build.json")));
  assert.equal(record.revision, owner.revision);
  assert.deepEqual(treeInputs(harness), record.inputs);
  const executable = realpathSync(record.executable);
  assert.ok(executable.startsWith(`${realpathSync(join(harness, "target"))}/debug/deps/`));
  assert.equal(binaryDigest(executable), record.binaryDigest);
  // Enter only this owned container's network namespace. No port is exposed;
  // retain the host filesystem so the exact host-built test uses its own libc.
  // setpriv drops UID/GID, supplemental groups and privilege gain before Rust.
  const log = unit(`chariox-browser-live-${owner.id}`, ["MemoryMax=256M", "CPUQuota=100%", "TasksMax=32", "RuntimeMaxSec=30"],
    "/usr/bin/nsenter", [`--net=/proc/${pid}/ns/net`, "--", "/usr/bin/setpriv", `--reuid=${uid}`, `--regid=${gid}`, "--clear-groups", "--no-new-privs",
      "/usr/bin/env", "-i", "PATH=/usr/bin:/bin", "CHARIOX_BROWSER_CONTROLLER_HOSTED=owned-production-browser", executable,
      "--ignored", "--exact", "browser_controller::tests::hosted::actual_chromium_exact_target_projection", "--test-threads=1", "--nocapture"], 40000);
  writeFileSync(join(scratch, "evidence/controller-live-tests.log"), log);
  assert.match(log, /1 passed; 0 failed/);
  const line = log.split("\n").find(value => value.includes("browser_controller_acceptance="));
  assert.ok(line);
  return { ...JSON.parse(line.split("browser_controller_acceptance=")[1]), sourceRevision: owner.revision,
    sourceDigest: digest(JSON.stringify(record.inputs)), memoryMaxBytes: 256 * 1024 ** 2, tasksMax: 32,
    runtimeMaxSeconds: 30, chromiumNamespaceOnly: true };
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.argv[2], "build");
  build(process.argv[3]);
}
