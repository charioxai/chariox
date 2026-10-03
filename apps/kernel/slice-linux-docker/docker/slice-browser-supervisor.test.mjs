import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm, access } from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import os from "node:os";
import path from "node:path";

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitFor(check, timeout = 12000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { if (await check()) return; await delay(50); }
  assert.fail("supervisor did not reach the expected state");
}

// The supervisor relaunches release F browser lifetimes: copy the production
// lifecycle owner beside the launcher and keep its records inside the fixture.
async function fixture(prefix) {
  const root = await mkdtemp(path.join(os.tmpdir(), prefix));
  await mkdir(path.join(root, "bin"));
  for (const name of ["slice-screen.sh", "browser-lifecycle.py", "browser-upload-store.py"]) {
    await copyFile(new URL(`./${name}`, import.meta.url), path.join(root, name));
  }
  const env = { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, CHARIOX_SLICE_ROOT: root,
    CHARIOX_SLICE_CHROME_PROFILE: `${root}/profile`, HOME: root, TMPDIR: root,
    CHARIOX_BROWSER_LIFECYCLE_ROOT: `${root}/lifetimes`, PYTHONDONTWRITEBYTECODE: "1", NODE_BINARY: process.execPath };
  return { root, env };
}

// Chromium is matched by its executable (never the lifecycle's Python owner,
// whose argv also carries the browser's). Keep the fake's argv[0] the browser.
async function fakeChromium(root, body, file = "fake-browser.sh", interpreter = "bash") {
  await writeFile(path.join(root, file), body);
  await writeFile(path.join(root, "bin/chromium"),
    `#!/usr/bin/env bash\nexec -a "$0" ${interpreter === "node" ? '"$NODE_BINARY"' : "bash"} "$CHARIOX_SLICE_ROOT/${file}" "$@"\n`, { mode: 0o755 });
}

const recordingBrowser = `set -eu
printf '%s\\n' "$$" >> "$CHARIOX_SLICE_ROOT/children"
printf '%s\\n' "$*" >> "$CHARIOX_SLICE_ROOT/arguments"
trap 'exit 0' TERM INT
while true; do sleep 0.1; done
`;

test("browser supervisor owns one lifetime, reaps TERM exits and restarts without losing the profile", { timeout: 45000 }, async () => {
  const { root, env } = await fixture("slice-browser-supervisor-");
  await fakeChromium(root, recordingBrowser);
  const supervisor = spawn("bash", [path.join(root, "slice-screen.sh"), "supervise-browser"], { env, stdio: "ignore" });
  const exited = once(supervisor, "exit");
  let child;
  try {
    await waitFor(async () => { try { child = Number((await readFile(path.join(root, "children"), "utf8")).trim()); return child > 0; } catch { return false; } });
    const duplicate = spawn("bash", [path.join(root, "slice-screen.sh"), "supervise-browser"], { env, stdio: "ignore" });
    assert.equal((await once(duplicate, "exit"))[0], 1);
    process.kill(child, "SIGTERM");
    const old = child;
    await waitFor(async () => { const children = (await readFile(path.join(root, "children"), "utf8")).trim().split("\n"); if (children.length !== 2) return false; child = Number(children[1]); return true; });
    assert.throws(() => process.kill(old, 0), { code: "ESRCH" }, "the previous browser must be reaped");
    assert.ok((await readFile(path.join(root, "arguments"), "utf8")).split("\n").filter(Boolean).every(line => line.includes(`--user-data-dir=${root}/profile`)));
    supervisor.kill("SIGTERM");
    assert.equal((await exited)[0], 0);
    assert.throws(() => process.kill(child, 0), { code: "ESRCH" }, "supervisor shutdown must retire its browser lifetime");
  } finally {
    if (supervisor.exitCode === null) { supervisor.kill("SIGKILL"); await exited; }
    if (child) { try { process.kill(child, "SIGKILL"); } catch {} }
    await rm(root, { recursive: true, force: true });
  }
});

test("open-url recovers backoff and stale supervisor PIDs without signalling another process", { timeout: 30000 }, async () => {
  const { root, env } = await fixture("slice-browser-backoff-");
  await fakeChromium(root, recordingBrowser);
  await writeFile(path.join(root, "bin/pgrep"), `#!/bin/sh
case "$*" in *Xvfb*) printf '123 Xvfb fixture\\n'; exit 0;; esac
exec /usr/bin/pgrep "$@"
`, { mode: 0o755 });
  for (const name of ["xdpyinfo", "timeout"]) await writeFile(path.join(root, "bin", name), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  const supervisor = spawn("bash", [path.join(root, "slice-screen.sh"), "supervise-browser"], { env, stdio: "ignore" });
  const exited = once(supervisor, "exit");
  let replacementPid, child, unrelated, unrelatedExit;
  try {
    await waitFor(async () => { try { child = Number((await readFile(path.join(root, "children"), "utf8")).trim()); return child > 0; } catch { return false; } });
    process.kill(child, "SIGTERM");
    // The initial supervisor was started directly, so its log is discarded;
    // its child must have been reaped before exercising the backoff window.
    await waitFor(async () => { try { process.kill(child, 0); return false; } catch { return true; } });
    const begin = Date.now();
    const result = spawnSync("bash", [path.join(root, "slice-screen.sh"), "open-url", "https://recovery.test/"], { env, encoding: "utf8", timeout: 12000 });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(Date.now() - begin < 5000, "backoff TERM must not wait for the six-second sleep");
    assert.equal((await exited)[0], 0);
    replacementPid = Number(await readFile(path.join(root, "logs/chromium-supervisor.pid"), "utf8"));
    assert.notEqual(replacementPid, supervisor.pid);
    assert.ok((await readFile(path.join(root, "arguments"), "utf8")).includes("--new-window -- https://recovery.test/"));
    process.kill(replacementPid, "SIGTERM");
    await waitFor(async () => { try { await readFile(path.join(root, "logs/chromium-supervisor.pid")); return false; } catch { return true; } });
    replacementPid = undefined;
    unrelated = spawn(process.execPath, ["-e", `process.on('SIGTERM',()=>require('node:fs').writeFileSync(${JSON.stringify(path.join(root, "unrelated-signalled"))},'TERM'));console.log('ready');setInterval(()=>{},1000)`], { stdio: ["ignore", "pipe", "ignore"] });
    unrelatedExit = once(unrelated, "exit");
    await once(unrelated.stdout, "data");
    await writeFile(path.join(root, "logs/chromium-supervisor.pid"), String(unrelated.pid));
    const restarted = spawnSync("bash", [path.join(root, "slice-screen.sh"), "open-url", "https://recovery.test/restarted"], { env, encoding: "utf8", timeout: 12000 });
    assert.equal(restarted.status, 0, restarted.stderr);
    process.kill(unrelated.pid, 0);
    await assert.rejects(readFile(path.join(root, "unrelated-signalled")), { code: "ENOENT" });
    replacementPid = Number(await readFile(path.join(root, "logs/chromium-supervisor.pid"), "utf8"));
    assert.notEqual(replacementPid, unrelated.pid);
  } finally {
    if (replacementPid) { try { process.kill(replacementPid, "SIGTERM"); } catch {} }
    if (supervisor.exitCode === null) { supervisor.kill("SIGTERM"); await exited; }
    await waitFor(async () => { try { await readFile(path.join(root, "logs/chromium-supervisor.pid")); return false; } catch { return true; } });
    if (unrelated) { unrelated.kill("SIGKILL"); await unrelatedExit; }
    await rm(root, { recursive: true, force: true });
  }
});

async function stopFixture(prefix) {
  const { root, env } = await fixture(prefix);
  await mkdir(path.join(root, "logs"));
  await mkdir(path.join(root, "profile"));
  await writeFile(path.join(root, "bin/pgrep"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
  await writeFile(path.join(root, "bin/pkill"), `#!/bin/sh
printf '%s\\n' "$*" >> "$CHARIOX_SLICE_ROOT/stopped-patterns"
`, { mode: 0o755 });
  return { root, env };
}

function assertTeardown(patterns) {
  for (const name of ["pulseaudio", "websockify", "x11vnc", "openbox", "tint2", "Xvfb"]) assert.ok(patterns.includes(name), `teardown must reach ${name}`);
  assert.ok(!patterns.includes("chromium"), "browsers are retired by their lifetime owner, never by pattern");
}

test("desktop stop retires a TERM-ignoring browser lifetime, completes teardown and clears profile locks", { timeout: 30000 }, async () => {
  const { root, env } = await stopFixture("slice-browser-stop-");
  await fakeChromium(root, `import {writeFileSync} from 'node:fs';
process.on('SIGTERM',()=>{});
writeFileSync(process.env.CHARIOX_SLICE_ROOT+'/browser-ready',String(process.pid));
setInterval(()=>{},1000);
`, "fake-browser.mjs", "node");
  const supervisor = spawn("bash", [path.join(root, "slice-screen.sh"), "supervise-browser"], { env, stdio: "ignore" });
  const exited = once(supervisor, "exit");
  let browserPid;
  try {
    await waitFor(async () => { try { browserPid = Number(await readFile(path.join(root, "browser-ready"), "utf8")); return browserPid > 0; } catch { return false; } });
    await writeFile(path.join(root, "profile/SingletonLock"), "fixture");
    const result = spawnSync("bash", [path.join(root, "slice-screen.sh"), "stop"], { env, encoding: "utf8", timeout: 25000 });
    assert.equal(result.status, 0, result.stderr);
    assert.doesNotMatch(result.stderr, /supervisor did not stop/);
    assert.equal((await exited)[0], 0);
    assert.throws(() => process.kill(browserPid, 0), { code: "ESRCH" }, "the lifetime owner escalates and reaps the browser");
    assertTeardown(await readFile(path.join(root, "stopped-patterns"), "utf8"));
    await assert.rejects(access(path.join(root, "profile/SingletonLock")), { code: "ENOENT" });
  } finally {
    if (browserPid) { try { process.kill(browserPid, "SIGKILL"); } catch {} }
    if (supervisor.exitCode === null) { supervisor.kill("SIGKILL"); await exited; }
    await rm(root, { recursive: true, force: true });
  }
});

test("desktop stop completes teardown but keeps profile locks when browser retirement is unproven", { timeout: 30000 }, async () => {
  const { root, env } = await stopFixture("slice-browser-unowned-");
  // A browser outside any owned lifetime: the lifecycle refuses to claim its
  // retirement and nothing may terminate it by pattern.
  const unowned = spawn(process.execPath, ["-e", "console.log('ready');setInterval(()=>{},1000)", "--", `--user-data-dir=${root}/profile`], { stdio: ["ignore", "pipe", "ignore"] });
  const unownedExit = once(unowned, "exit");
  try {
    await once(unowned.stdout, "data");
    await writeFile(path.join(root, "profile/SingletonLock"), "fixture");
    const result = spawnSync("bash", [path.join(root, "slice-screen.sh"), "stop"], { env, encoding: "utf8", timeout: 25000 });
    assert.equal(result.status, 1, result.stderr);
    assertTeardown(await readFile(path.join(root, "stopped-patterns"), "utf8"));
    assert.equal(await readFile(path.join(root, "profile/SingletonLock"), "utf8"), "fixture");
    process.kill(unowned.pid, 0);
  } finally {
    unowned.kill("SIGKILL");
    await unownedExit;
    await rm(root, { recursive: true, force: true });
  }
});
