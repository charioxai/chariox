import assert from "node:assert/strict";
import { appendFileSync, chmodSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { checked, cleanup, docker, loadOwner } from "./resources.mjs";
import { source } from "./prepare.mjs";

const [mode, scratch] = process.argv.slice(2);
const owner = loadOwner(scratch);
if (mode === "cleanup") { cleanup(scratch); process.exit(0); }
assert.equal(mode, "run");
const label = `io.chariox.chromium-drill=${owner.id}`;
const evidence = join(scratch, "evidence");
const policy = join(scratch, "chromium-seccomp.json");
const results = [];
const containers = [];
const record = (name, value) => { results.push({ name, ...value }); writeFileSync(join(evidence, "results.json"), JSON.stringify(results, null, 2)); };
const execute = (id, args, seconds = 60, user = "slice") => docker(["exec", "-u", user, id, "timeout", "--kill-after=2s", `${seconds}s`, ...args], { timeout: (seconds + 10) * 1000 });
const volume = role => docker(["volume", "create", "--label", label, `chariox-chromium-${owner.id}-${role}`]);
function create(role, home, extra = []) {
  const id = docker(["create", "--name", `chariox-chromium-${owner.id}-${role}`, "--label", label,
    "--network", "none", "--cpus", "2", "--memory", "2g", "--memory-swap", "2g", "--pids-limit", "512",
    "--shm-size", "128m", "--ulimit", "core=0:0", "--security-opt", `seccomp=${policy}`,
    ...(home ? ["-v", `${home}:/home/slice`] : []), ...extra, owner.image]);
  assert.match(id, /^[a-f0-9]{64}$/);
  containers.push(id);
  return id;
}
function startBrowser(id, name) {
  docker(["start", id]);
  execute(id, ["bash", "/opt/chariox-slice/slice-screen.sh", "start"], 90);
  const sandbox = JSON.parse(execute(id, ["node", "/opt/chariox-slice/chromium-sandbox-probe.mjs"], 30));
  assert.equal(sandbox.chromiumSandboxVerified, true);
  const config = JSON.parse(docker(["inspect", "--format", "{{json .HostConfig}}", id]));
  assert.equal(config.Memory, 2 * 1024 ** 3);
  assert.equal(config.MemorySwap, config.Memory);
  assert.equal(config.NanoCpus, 2 * 10 ** 9);
  assert.equal(config.PidsLimit, 512);
  assert.equal(config.Privileged, false);
  assert.equal(config.NetworkMode, "none");
  assert.deepEqual(JSON.parse(config.SecurityOpt.find(option => option.startsWith("seccomp=")).slice(8)), JSON.parse(readFileSync(policy)));
  record(name, { ...sandbox, resourcesVerified: true });
}
function profile(id, action, name) {
  const result = JSON.parse(execute(id, ["node", "/opt/chariox-drill/profile.mjs", action], 30));
  record(name, result);
  return result;
}

try {
  const firstVolume = volume("source");
  const first = create("source", firstVolume);
  startBrowser(first, "initial-sandbox");
  writeFileSync(join(evidence, "versions.txt"), execute(first, ["bash", "-lc", "node --version; chromium --version; dpkg-query -W chromium chromium-sandbox; uname -r"]));
  profile(first, "seed", "seed");
  profile(first, "verify", "initial-storage");
  execute(first, ["bash", "/opt/chariox-slice/slice-screen.sh", "stop"], 90);
  assert.equal(execute(first, ["bash", "-lc", "pgrep -af '/usr/lib/chromium/chromium' | grep -v pgrep | grep -v defunct || true"]), "");
  // The archive contains the whole stopped home, including Local State,
  // cookies and session metadata, using the production tar.zst contract.
  execute(first, ["bash", "-lc", "cd /home/slice; tar --zstd -cf /tmp/home.tar.zst ."], 60);
  const archive = join(scratch, "home.tar.zst");
  docker(["cp", `${first}:/tmp/home.tar.zst`, archive]);
  assert.ok(statSync(archive).size > 0 && statSync(archive).size <= 256 * 1024 ** 2);
  docker(["stop", "--time", "10", first]);
  docker(["rm", first]);

  const restoredVolume = volume("restored");
  // Exercise the actual production restore action. Its Docker create calls get
  // additional hosted resource limits and our cleanup label through this shim.
  writeFileSync(join(scratch, "bin/docker"), `#!/usr/bin/env bash\nset -euo pipefail\nif [[ "$1" == create ]]; then shift; exec /usr/bin/docker create --network none --memory 512m --memory-swap 512m --cpus 1 --pids-limit 128 --label '${label}' "$@"; fi\nexec /usr/bin/docker "$@"\n`);
  chmodSync(join(scratch, "bin/docker"), 0o700);
  checked("bash", [join(source, "provision-linux-docker-slice.sh"), "restore-migration-home"], { timeout: 360000, env: {
    PATH: `${join(scratch, "bin")}:${process.env.PATH}`, HOME: join(scratch, "home"), TMPDIR: join(scratch, "tmp"),
    CHARIOX_SLICE_NAME: `chariox-chromium-${owner.id}`, CHARIOX_SLICE_HOME_VOLUME: restoredVolume,
    CHARIOX_SLICE_DOCKER_IMAGE: owner.image, CHARIOX_SLICE_SAVED_HOME_ARCHIVE: archive,
    CHARIOX_SLICE_CHROMIUM_MIGRATION_ID: owner.id,
  } });
  const restored = create("restored", restoredVolume);
  startBrowser(restored, "restored-sandbox");
  const restoredState = profile(restored, "verify", "restored-storage");
  assert.equal(restoredState.sessionTabRestored, true, "the production launcher did not restore the saved fixture tab");
  profile(restored, "revoked", "server-revocation-negative");
  execute(restored, ["bash", "/opt/chariox-slice/slice-screen.sh", "stop"], 90);
  docker(["stop", "--time", "10", restored]);
  docker(["rm", restored]);

  const empty = create("empty", volume("empty"));
  startBrowser(empty, "empty-profile-sandbox");
  profile(empty, "empty", "storage-loss-negative");
  record("acceptance", { productionLauncherFixture: true, fullKernelMigrationValidated: false, googleAuthenticationValidated: false });
} catch (error) {
  writeFileSync(join(evidence, "failure.txt"), String(error.stack ?? error).slice(0, 32768));
  for (const id of containers) {
    try { writeFileSync(join(evidence, `${id.slice(0, 12)}-browser.log`), execute(id, ["tail", "-c", "32768", "/opt/chariox-slice/logs/chromium-gui.log"], 5)); } catch {}
  }
  throw error;
} finally {
  try { cleanup(scratch); } catch (error) { appendFileSync(join(evidence, "cleanup.txt"), String(error).slice(0, 8192)); throw error; }
}
