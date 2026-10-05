import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { appendFileSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { restoreScript } from "./restore.mjs";
import { drillEnvironment } from "./environment.mjs";

export const repository = fileURLToPath(new URL("../../", import.meta.url));
export const source = join(repository, "apps/kernel/slice-linux-docker");
export const fixtureSource = dirname(fileURLToPath(import.meta.url));
// The launcher starts and retires Chromium through the release F lifetime owner;
// its verified Browser.close imports the controller CDP client (closure below).
const browserCloseModules = [
  "browser-controller-actions.mjs",
  "browser-controller-actionability.mjs",
  "browser-controller-interactions.mjs",
  "browser-controller-geometry.mjs",
  "browser-controller-bar.mjs",
  "browser-controller-cdp.mjs",
  "browser-controller-compatibility.mjs",
  "browser-controller-cookie-fence.mjs",
  "browser-controller-dialogs.mjs",
  "browser-controller-events.mjs",
  "browser-controller-files.mjs",
  "browser-controller-frames.mjs",
  "browser-controller-history.mjs",
  "browser-controller-input.mjs",
  "browser-controller-permissions.mjs",
  "browser-controller-snapshot.mjs",
  "browser-controller-text.mjs",
  "browser-controller-selection.mjs",
  "browser-controller-upload-staging.mjs",
];
// slice-screen.sh puts saved App navigations behind placeholders before an
// owned launch restores the profile's session.
const productionFiles = ["docker/slice-screen.sh", "docker/browser-app-restore.mjs", "docker/browser-cdp.mjs", "docker/browser-lifecycle.py",
  "docker/browser-upload-store.py", ...browserCloseModules.map(name => `docker/${name}`), "docker/tint2rc", "chromium-seccomp.json"];
const launcherFiles = productionFiles.filter(name => name.startsWith("docker/")).map(name => name.slice("docker/".length));
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
export function verifyInputs({ sourceRoot = source, fixtureRoot = fixtureSource } = {}) {
  const production = readFileSync(join(sourceRoot, "docker/Dockerfile"), "utf8");
  const fixture = readFileSync(join(fixtureRoot, "Dockerfile"), "utf8");
  for (const pattern of [/^FROM node:22\.17\.1-bookworm@sha256:[a-f0-9]{64} AS ca-bundle$/m,
    /^RUN echo '[a-f0-9]{64}  \/etc\/ssl\/certs\/ca-certificates\.crt'.*$/m,
    /^FROM node:22-bookworm-slim@sha256:[a-f0-9]{64}$/m,
    /https:\/\/snapshot\.debian\.org\/archive\/debian\/\d{8}T\d{6}Z/]) {
    assert.ok(production.match(pattern), "production immutable browser input is missing");
    assert.equal(fixture.match(pattern)?.[0], production.match(pattern)[0], "fixture browser inputs drifted from production");
  }
  const browserStage = production.slice(production.lastIndexOf("FROM node:22-bookworm-slim@"));
  for (const text of [browserStage, fixture]) {
    assert.match(text, /^COPY --from=ca-bundle \/etc\/ssl\/certs\/ca-certificates\.crt \/etc\/ssl\/certs\/ca-certificates\.crt$/m, "CA bundle copy is missing");
  }
  const aptSetup = /RUN printf[^]*?&& apt-get install -y --no-install-recommends --allow-downgrades/;
  assert.ok(browserStage.match(aptSetup), "production browser apt setup is missing");
  assert.equal(fixture.match(aptSetup)?.[0], browserStage.match(aptSetup)[0], "fixture apt setup drifted from production");
  const packages = text => text.match(/apt-get install -y --no-install-recommends --allow-downgrades([^]*?)&&/)[1]
    .replace(/\\/g, "").trim().split(/\s+/);
  const browserPackages = "bash chromium chromium-sandbox curl dbus fonts-dejavu fonts-liberation novnc openbox procps python3 tint2 websockify x11-utils x11vnc xdotool xvfb zstd".split(" ");
  assert.deepEqual(packages(fixture).sort(), browserPackages.sort(), "fixture apt package selection drifted");
  for (const name of packages(fixture)) assert.ok(packages(browserStage).includes(name), `fixture apt package drifted: ${name}`);
  for (const name of launcherFiles) {
    assert.ok(browserStage.split("\n").some(line => /^(COPY|COPY --chown=slice:slice) /.test(line)
      && line.endsWith(`apps/kernel/slice-linux-docker/docker/${name} /opt/chariox-slice/${name}`)), `production image omits ${name}`);
  }
  // A new import of the owned close path must also reach the fixture image.
  const closure = new Set(), pending = ["browser-cdp.mjs"];
  while (pending.length > 0) {
    const name = pending.pop();
    if (closure.has(name)) continue;
    closure.add(name);
    for (const match of readFileSync(join(sourceRoot, "docker", name), "utf8").matchAll(/(?:from|import\()\s*["']\.\/([^"'/]+\.mjs)["']/g)) pending.push(match[1]);
  }
  for (const name of closure) assert.ok(launcherFiles.includes(name), `fixture omits browser-cdp dependency ${name}`);
  const fixtureCopy = fixture.match(/^COPY (.+) \/opt\/chariox-slice\/$/m)?.[1].split(" ") ?? [];
  assert.deepEqual([...fixtureCopy].sort(), [...launcherFiles].sort(), "fixture launcher COPY drifted from the pinned production files");
  const pins = JSON.parse(readFileSync(join(fixtureRoot, "inputs.lock.json")));
  assert.deepEqual(Object.keys(pins).sort(), [...productionFiles, "restore-script"].sort());
  for (const name of productionFiles) {
    assert.match(pins[name], /^[a-f0-9]{64}$/);
    assert.equal(digest(readFileSync(join(sourceRoot, name))), pins[name], `production input hash drifted: ${name}`);
  }
  assert.equal(digest(restoreScript(readFileSync(join(sourceRoot, "provision-linux-docker-slice.sh"), "utf8"))), pins["restore-script"], "production restore input hash drifted");
  return pins;
}

export function prepare() {
  const executionEnvironment = drillEnvironment();
  const pins = verifyInputs();
  const revision = spawnSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8", timeout: 3000 }).stdout.trim();
  assert.match(revision, /^[a-f0-9]{40}$/);
  assert.equal(revision, process.env.EXPECTED_REVISION);
  const scratch = mkdtempSync(join(realpathSync(process.env.RUNNER_TEMP), "chariox-chromium."));
  const context = join(scratch, "context");
  const evidence = join(scratch, "evidence");
  for (const path of [context, evidence, join(scratch, "home"), join(scratch, "tmp"), join(scratch, "bin")]) mkdirSync(path, { mode: 0o700 });
  const id = randomBytes(12).toString("hex");
  const manifest = { id, revision, image: `chariox-chromium-drill:${id}`, executionEnvironment, inputs: { ...pins,
    productionDockerfile: digest(readFileSync(join(source, "docker/Dockerfile"))),
    fixtureDockerfile: digest(readFileSync(join(fixtureSource, "Dockerfile"))),
  }, testOnlyInputs: {} };
  for (const name of productionFiles.filter(name => name.startsWith("docker/"))) {
    const path = join(source, name);
    assert.equal(digest(readFileSync(path)), pins[name]);
    copyFileSync(path, join(context, name.slice("docker/".length)));
    assert.equal(digest(readFileSync(join(context, name.slice("docker/".length)))), pins[name]);
  }
  // This retained source probe is a test fixture; production no longer ships it.
  const probe = join(source, "docker/chromium-sandbox-probe.mjs");
  copyFileSync(probe, join(context, "chromium-sandbox-probe.mjs"));
  manifest.testOnlyInputs["chromium-sandbox-probe.mjs"] = digest(readFileSync(probe));
  for (const name of ["Dockerfile", "fixture-server.mjs", "cdp.mjs", "profile.mjs"]) copyFileSync(join(fixtureSource, name), join(context, name));
  copyFileSync(join(source, "chromium-seccomp.json"), join(scratch, "chromium-seccomp.json"));
  assert.equal(digest(readFileSync(join(scratch, "chromium-seccomp.json"))), pins["chromium-seccomp.json"]);
  writeFileSync(join(scratch, "owner.json"), JSON.stringify(manifest), { mode: 0o600 });
  writeFileSync(join(evidence, "inputs.json"), JSON.stringify(manifest, null, 2));
  appendFileSync(process.env.GITHUB_OUTPUT, `scratch=${scratch}\ncontext=${context}\nevidence=${evidence}\nimage=${manifest.image}\nid=${id}\n`);
  return scratch;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) prepare();
