import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { appendFileSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

export const repository = fileURLToPath(new URL("../../", import.meta.url));
export const source = join(repository, "apps/kernel/slice-linux-docker");
export function verifyInputs() {
  const production = readFileSync(join(source, "docker/Dockerfile"), "utf8");
  const fixture = readFileSync(new URL("./Dockerfile", import.meta.url), "utf8");
  for (const pattern of [/^FROM node:22\.17\.1-bookworm@sha256:[a-f0-9]{64} AS ca-bundle$/m,
    /^RUN echo '[a-f0-9]{64}  \/etc\/ssl\/certs\/ca-certificates\.crt'.*$/m,
    /^FROM node:22-bookworm-slim@sha256:[a-f0-9]{64}$/m,
    /https:\/\/snapshot\.debian\.org\/archive\/debian\/\d{8}T\d{6}Z/]) {
    assert.ok(production.match(pattern), "production immutable browser input is missing");
    assert.equal(fixture.match(pattern)?.[0], production.match(pattern)[0], "fixture browser inputs drifted from production");
  }
  for (const name of ["slice-screen.sh", "browser-cdp.mjs", "chromium-sandbox-probe.mjs"]) {
    assert.ok(production.includes(`apps/kernel/slice-linux-docker/docker/${name} /opt/chariox-slice/${name}`), `production image omits ${name}`);
  }
}

export function prepare() {
  assert.equal(process.env.GITHUB_ACTIONS, "true", "the real browser drill runs on its hosted runner");
  verifyInputs();
  const revision = spawnSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8", timeout: 3000 }).stdout.trim();
  assert.match(revision, /^[a-f0-9]{40}$/);
  assert.equal(revision, process.env.EXPECTED_REVISION);
  const scratch = mkdtempSync(join(realpathSync(process.env.RUNNER_TEMP), "chariox-chromium."));
  const context = join(scratch, "context");
  const evidence = join(scratch, "evidence");
  for (const path of [context, evidence, join(scratch, "home"), join(scratch, "tmp"), join(scratch, "bin")]) mkdirSync(path, { mode: 0o700 });
  const id = randomBytes(12).toString("hex");
  const manifest = { id, revision, image: `chariox-chromium-drill:${id}`, inputs: {} };
  for (const name of ["slice-screen.sh", "browser-cdp.mjs", "chromium-sandbox-probe.mjs"]) {
    const path = join(source, "docker", name);
    copyFileSync(path, join(context, name));
    manifest.inputs[name] = createHash("sha256").update(readFileSync(path)).digest("hex");
  }
  for (const name of ["Dockerfile", "fixture-server.mjs", "cdp.mjs", "profile.mjs"]) copyFileSync(new URL(`./${name}`, import.meta.url), join(context, name));
  copyFileSync(join(source, "chromium-seccomp.json"), join(scratch, "chromium-seccomp.json"));
  manifest.inputs.seccomp = createHash("sha256").update(readFileSync(join(scratch, "chromium-seccomp.json"))).digest("hex");
  writeFileSync(join(scratch, "owner.json"), JSON.stringify(manifest), { mode: 0o600 });
  writeFileSync(join(evidence, "inputs.json"), JSON.stringify(manifest, null, 2));
  appendFileSync(process.env.GITHUB_OUTPUT, `scratch=${scratch}\ncontext=${context}\nevidence=${evidence}\nimage=${manifest.image}\nid=${id}\n`);
  return scratch;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) prepare();
