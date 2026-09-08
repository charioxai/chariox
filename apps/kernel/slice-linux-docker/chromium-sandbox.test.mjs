import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { fixture, provisionScript, provisionSource, screenScript, screenSource, shellFunction } from "./chromium-shell-fixture.mjs";

const policyBytes = readFileSync(new URL("./chromium-seccomp.json", import.meta.url));
const policyDigest = createHash("sha256").update(policyBytes).digest("hex");
const scriptDir = dirname(fileURLToPath(import.meta.url));

function success(result) {
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
}

function browserContract(args, profile) {
  assert.ok(args, "Chromium was not launched");
  assert.equal(args[0], "chromium");
  const options = args.slice(0, args.indexOf("--") === -1 ? undefined : args.indexOf("--"));
  assert.ok(options.includes(`--user-data-dir=${profile}`));
  assert.ok(options.includes("--password-store=basic"));
  assert.ok(options.includes("--remote-debugging-address=127.0.0.1"));
  assert.ok(options.includes("--remote-debugging-port=9222"));
  for (const option of ["--no-sandbox", "--disable-setuid-sandbox", "--disable-seccomp-filter-sandbox"]) assert.ok(!options.includes(option));
}

test("the pinned Docker default-deny policy changes only three namespace syscalls", () => {
  const profile = JSON.parse(policyBytes);
  const namespaces = profile.syscalls.pop();
  assert.deepEqual(namespaces.names, ["clone", "setns", "unshare"]);
  assert.equal(namespaces.action, "SCMP_ACT_ALLOW");
  assert.equal(profile.defaultAction, "SCMP_ACT_ERRNO");
  assert.equal(createHash("sha256").update(JSON.stringify(profile)).digest("hex"),
    "afb4934b023cfceaaec1a9d752ca3f801aaa96eb2e59abe6e7ea16976948e080");
  for (const source of [screenSource, provisionSource]) assert.doesNotMatch(source, /--(?:no-sandbox|disable-setuid-sandbox|disable-seccomp-filter-sandbox)/);
});

test("default profile remains the existing .config path", () => {
  const f = fixture();
  try {
    const result = f.run(screenScript('printf "%s" "$CHROME_PROFILE"'), { CHARIOX_SLICE_CHROME_PROFILE: "" });
    success(result);
    assert.equal(result.stdout, join(process.env.HOME, ".config/chariox-slice-chromium"));
  } finally { f.cleanup(); }
});

test("desktop startup uses the shared sandboxed launcher and keeps stored profile bytes", () => {
  const f = fixture();
  try {
    const preserved = ["Local State", "Default/Cookies", "Default/Login Data", "Default/Web Data"];
    for (const name of preserved) writeFileSync(join(f.profile, name), `fixture:${name}`);
    const result = f.run(screenScript("start_desktop"), { FIXTURE_DISPLAY: "0" });
    success(result);
    const calls = f.calls();
    browserContract(calls.chromium, f.profile);
    assert.deepEqual(calls.chromium.slice(-2), ["--", "about:blank"]);
    for (const process of ["Xvfb", "openbox", "x11vnc", "websockify"]) assert.ok(calls[process]);
    for (const name of preserved) assert.equal(readFileSync(join(f.profile, name), "utf8"), `fixture:${name}`);
  } finally { f.cleanup(); }
});

for (const session of ["Sessions/Session_42", "Last Session", "Current Session"]) {
  test(`nonempty ${session} restores tabs on startup and cold recovery`, () => {
    const f = fixture();
    try {
      mkdirSync(join(f.profile, "Default/Sessions"), { recursive: true });
      writeFileSync(join(f.profile, "Default", session), "session fixture");
      success(f.run(screenScript("launch_chromium")));
      assert.ok(f.calls().chromium.includes("--restore-last-session"));
      assert.ok(!f.calls().chromium.includes("about:blank"), "do not add a blank tab to a restored session");
      unlinkSync(join(f.capture, "chromium")); // Simulate the original browser having exited.
      success(f.run(screenScript('open_url "https://example.test/recovery"')));
      const calls = f.calls();
      browserContract(calls.chromium, f.profile);
      assert.ok(calls.chromium.includes("--restore-last-session"));
      assert.deepEqual(calls.chromium.slice(-3), ["--new-window", "--", "https://example.test/recovery"]);
      assert.ok(!calls.Xvfb && !calls.stop, "cold browser recovery must leave the desktop running");
    } finally { f.cleanup(); }
  });
}

test("empty session files and tab files alone do not request restore", () => {
  const f = fixture();
  try {
    mkdirSync(join(f.profile, "Default/Sessions"));
    writeFileSync(join(f.profile, "Default/Sessions/Session_1"), "");
    writeFileSync(join(f.profile, "Default/Sessions/Tabs_1"), "tabs fixture");
    writeFileSync(join(f.profile, "Default/Last Session"), "");
    success(f.run(screenScript("launch_chromium")));
    assert.ok(!f.calls().chromium.includes("--restore-last-session"));
  } finally { f.cleanup(); }
});

test("warm fallback retains live locks and preferences, and treats a flag-shaped URL as data", () => {
  const f = fixture();
  try {
    const prefs = join(f.profile, "Default/Preferences");
    const lock = join(f.profile, "SingletonLock");
    writeFileSync(prefs, '{"keep":"exact bytes"}\n');
    writeFileSync(lock, "active lock");
    success(f.run(screenScript('open_url "--no-sandbox"'), { FIXTURE_CHROMIUM: "1" }));
    const calls = f.calls();
    browserContract(calls.chromium, f.profile);
    assert.deepEqual(calls.chromium.slice(-3), ["--new-window", "--", "--no-sandbox"]);
    assert.deepEqual(calls.cdp.slice(0, 4), ["--kill-after=2s", "10s", "node", join(f.root, "browser-cdp.mjs")]);
    assert.equal(readFileSync(prefs, "utf8"), '{"keep":"exact bytes"}\n');
    assert.equal(readFileSync(lock, "utf8"), "active lock");
    assert.ok(!calls.stop && !calls.Xvfb);
  } finally { f.cleanup(); }
});

test("a successful CDP navigation focuses the existing browser without a new launch", () => {
  const f = fixture();
  try {
    success(f.run(screenScript('open_url "https://example.test"'), { FIXTURE_CDP_STATUS: "0", FIXTURE_CHROMIUM: "1" }));
    assert.ok(f.calls().focus);
    assert.ok(!f.calls().chromium);
  } finally { f.cleanup(); }
});

test("a missing display cannot be silently recreated by URL recovery", () => {
  const f = fixture();
  try {
    const result = f.run(screenScript('open_url "https://example.test"'), { FIXTURE_DISPLAY: "0" });
    assert.equal(result.status, 1);
    assert.deepEqual(f.calls(), {});
  } finally { f.cleanup(); }
});

test("regex characters in an active profile path cannot trigger cold cleanup", () => {
  const f = fixture();
  try {
    const profile = join(f.root, "profile [work](1)+.test");
    mkdirSync(join(profile, "Default"), { recursive: true });
    const lock = join(profile, "SingletonLock");
    writeFileSync(lock, "active");
    const script = `${shellFunction(screenSource, "process_running")}
pgrep() { printf '%s\\n' "123 chromium --user-data-dir=$CHROME_PROFILE --remote-debugging-port=9222" | grep -E "$2"; }
launch_chromium "https://example.test"`;
    success(f.run(screenScript(script), { CHARIOX_SLICE_CHROME_PROFILE: profile }));
    assert.equal(readFileSync(lock, "utf8"), "active");
    assert.ok(!existsSync(join(profile, "Default/Preferences")));
  } finally { f.cleanup(); }
});

for (const failure of [{ FIXTURE_LAUNCH_SURVIVES: "0" }, { FIXTURE_READY_STATUS: "1" }]) {
  test(`cold recovery reports failed readiness: ${JSON.stringify(failure)}`, () => {
    const f = fixture();
    try {
      const result = f.run(screenScript('open_url "https://example.test"'), failure);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Chromium did not become ready/);
      assert.ok(!f.calls().focus);
    } finally { f.cleanup(); }
  });
}

test("the actual root entrypoint cannot create profile or log directories", () => {
  const f = fixture();
  try {
    const bin = join(f.root, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "id"), "#!/bin/sh\nprintf '0\\n'\n");
    chmodSync(join(bin, "id"), 0o700);
    const profile = join(f.root, "must-not-exist-profile");
    const runtime = join(f.root, "must-not-exist-runtime");
    const script = fileURLToPath(new URL("./docker/slice-screen.sh", import.meta.url));
    const result = f.run('bash "$FIXTURE_SCREEN_SCRIPT" start', { PATH: `${bin}:${process.env.PATH}`,
      FIXTURE_SCREEN_SCRIPT: script, CHARIOX_SLICE_ROOT: runtime, CHARIOX_SLICE_CHROME_PROFILE: profile });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /non-root slice user/);
    assert.ok(!existsSync(profile) && !existsSync(runtime));
  } finally { f.cleanup(); }
});

for (const operation of ["start_desktop", "launch_chromium", 'open_url "https://example.test"']) {
  test(`root UID is rejected before ${operation} changes the browser`, () => {
    const f = fixture();
    try {
      const lock = join(f.profile, "SingletonLock");
      writeFileSync(lock, "preserve");
      const result = f.run(screenScript(operation), { FIXTURE_UID: "0", FIXTURE_CHROMIUM: "1" });
      assert.equal(result.status, 1);
      assert.match(result.stderr, /non-root slice user/);
      assert.deepEqual(f.calls(), {});
      assert.equal(readFileSync(lock, "utf8"), "preserve");
      assert.ok(!existsSync(join(f.profile, "Default/Preferences")));
    } finally { f.cleanup(); }
  });
}

test("shutdown bounds a stalled CDP close before process cleanup", () => {
  const f = fixture();
  try {
    success(f.run(screenScript(`${shellFunction(screenSource, "stop_desktop")}\nstop_desktop`), { FIXTURE_CHROMIUM: "1", FIXTURE_CDP_STATUS: "124" }));
    const calls = f.calls();
    assert.deepEqual(calls.cdp.slice(0, 3), ["--kill-after=2s", "10s", "node"]);
    assert.equal(calls.cdp.at(-1), "close-browser");
    assert.ok(calls.pkill && calls["stop-pattern"], "cleanup must follow failed/timed out CDP");
  } finally { f.cleanup(); }
});

function provisionEnvironment(extra = {}) {
  return { FIXTURE_SCRIPT_DIR: scriptDir, FIXTURE_EXISTS: "0", FIXTURE_POLICY: policyDigest, ...extra };
}

test("ordinary creation selects the pinned policy without broad capabilities", () => {
  const f = fixture();
  try {
    success(f.run(provisionScript("ensure_container 1"), provisionEnvironment()));
    const args = f.calls().create;
    assert.ok(args.includes(`seccomp=${join(scriptDir, "chromium-seccomp.json")}`));
    assert.ok(args.includes(`io.chariox.chromium-seccomp=${policyDigest}`));
    assert.ok(!args.some(arg => /unconfined|SYS_ADMIN|--privileged|--cap-add/.test(arg)));
  } finally { f.cleanup(); }
});

for (const scenario of [
  { name: "browser activation", command: "ensure_container 1", env: {} },
  { name: "forced recreation", command: "ensure_container", env: { FIXTURE_RECREATE: "1" } },
  { name: "stale image recreation", command: "ensure_container", env: { FIXTURE_IMAGE_ID: "new-image-id" } },
]) {
  test(`legacy policy preserves the live container before ${scenario.name}`, () => {
    const f = fixture();
    try {
      const result = f.run(provisionScript(scenario.command), provisionEnvironment({ FIXTURE_EXISTS: "1", FIXTURE_POLICY: "", ...scenario.env }));
      assert.equal(result.status, 1);
      assert.match(result.stderr, /slice\.chromium_sandbox_migration_required/);
      const calls = f.calls();
      assert.ok(!calls.create && !calls.restore);
      assert.ok(!calls.docker.some(arg => ["rm", "exec", "start", "create", "volume"].includes(arg)));
    } finally { f.cleanup(); }
  });
}

test("inspection failure cannot trigger destructive recreation", () => {
  const f = fixture();
  try {
    const result = f.run(provisionScript("ensure_container"), provisionEnvironment({ FIXTURE_EXISTS: "1", FIXTURE_RECREATE: "1", FIXTURE_INSPECT_FAILURE: "1" }));
    assert.equal(result.status, 1);
    assert.match(result.stderr, /failed to inspect/);
    assert.ok(!f.calls().docker.includes("rm"));
  } finally { f.cleanup(); }
});

test("legacy non-browser reuse remains available without migrating or replacing the container", () => {
  const f = fixture();
  try {
    success(f.run(provisionScript("ensure_container"), provisionEnvironment({ FIXTURE_EXISTS: "1", FIXTURE_POLICY: "" })));
    assert.ok(!f.calls().create && !f.calls().restore);
    assert.ok(!f.calls().docker.some(arg => arg.includes("io.chariox.chromium-seccomp")));
  } finally { f.cleanup(); }
});

test("compatible containers retain ordinary requested recreation", () => {
  const f = fixture();
  try {
    success(f.run(provisionScript("ensure_container 1"), provisionEnvironment({ FIXTURE_EXISTS: "1", FIXTURE_RECREATE: "1" })));
    assert.ok(f.calls().docker.includes("rm"));
    assert.ok(f.calls().restore && f.calls().create);
  } finally { f.cleanup(); }
});

test("migration recreation removes the validated immutable replacement ID", () => {
  const f = fixture();
  try {
    const id = "a".repeat(64);
    success(f.run(provisionScript('SLICE_CHROMIUM_MIGRATION_ID=checkpoint\nensure_container 1'), provisionEnvironment({
      FIXTURE_EXISTS: "1", FIXTURE_RECREATE: "1", FIXTURE_MIGRATION_METADATA: `${id}\ncheckpoint\nvolume:fixture-home`,
    })));
    const calls = f.calls().docker;
    const removed = calls.indexOf("rm");
    assert.deepEqual(calls.slice(removed, removed + 3), ["rm", "-f", id]);
  } finally { f.cleanup(); }
});

for (const ownership of ["other-checkpoint\nvolume:fixture-home", "checkpoint\nvolume:unrelated-home"]) {
  test(`migration rejects unrelated replacement before recreation: ${ownership.replaceAll("\n", "/")}`, () => {
    const f = fixture();
    try {
      const result = f.run(provisionScript('SLICE_CHROMIUM_MIGRATION_ID=checkpoint\nensure_container 1'), provisionEnvironment({
        FIXTURE_EXISTS: "1", FIXTURE_RECREATE: "1", FIXTURE_MIGRATION_METADATA: `${"a".repeat(64)}\n${ownership}`,
      }));
      assert.equal(result.status, 1);
      assert.match(result.stderr, /ownership differs/);
      assert.ok(!f.calls().docker.includes("rm"));
      assert.ok(!f.calls().create && !f.calls().restore);
    } finally { f.cleanup(); }
  });
}

test("explicit managed-provider policy remains separate from ordinary Chromium policy", () => {
  const f = fixture();
  try {
    success(f.run(provisionScript("ensure_container 1"), provisionEnvironment({ FIXTURE_UNCONFINED: "1" })));
    const args = f.calls().create;
    for (const option of ["seccomp=unconfined", "apparmor=unconfined", "systempaths=unconfined"]) assert.ok(args.includes(option));
    assert.ok(!args.some(arg => arg.includes("io.chariox.chromium-seccomp")));
  } finally { f.cleanup(); }
});
