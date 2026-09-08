import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fixture, provisionScript, provisionSource, shellFunction } from "./chromium-shell-fixture.mjs";

const helperId = "a".repeat(64);
function restoreScript() {
  return provisionScript(`${shellFunction(provisionSource, "recover_migration_restore_helpers")}
${shellFunction(provisionSource, "restore_saved_home_volume")}
SLICE_SAVED_HOME_ARCHIVE="$FIXTURE_ARCHIVE"
SLICE_CHROMIUM_MIGRATION_ID="$FIXTURE_MIGRATION_ID"
docker() {
  printf '%s\\0' "$@" >> "$FIXTURE_CAPTURE/restore-docker"
  if [[ "$1" == "$FIXTURE_FAIL" ]]; then return 1; fi
  case "$1" in
    ps) if [[ "$2" == -a ]]; then printf '%s\\n' "$FIXTURE_LEFTOVER"; else printf '%s\\n' "$FIXTURE_HOLDER"; fi ;;
    inspect) printf '%s\\n' "$FIXTURE_METADATA" ;;
    create) printf '%s\\n' "$FIXTURE_HELPER_ID" ;;
    exec) local argument; for argument; do :; done; printf '%s' "$argument" > "$FIXTURE_CAPTURE/restore-body" ;;
    start|cp|rm) : ;;
    *) return 1 ;;
  esac
}
restore_saved_home_volume`);
}

function captureRestore(f, failure = "none", createdId = helperId, extra = {}) {
  const archive = join(f.root, "checkpoint archive.tar.zst");
  writeFileSync(archive, "fixture archive");
  return f.run(restoreScript(), { FIXTURE_SCRIPT_DIR: f.root, FIXTURE_ARCHIVE: archive,
    FIXTURE_FAIL: failure, FIXTURE_HELPER_ID: createdId, FIXTURE_LEFTOVER: "", FIXTURE_HOLDER: "", FIXTURE_MIGRATION_ID: "owned-checkpoint",
    FIXTURE_METADATA: "owned-checkpoint\nfixture\nvolume:fixture-home", ...extra });
}

test("retry removes only an identity-checked interrupted helper before restoration", () => {
  const f = fixture();
  try {
    const leftover = "b".repeat(64);
    const result = captureRestore(f, "none", helperId, { FIXTURE_LEFTOVER: leftover });
    assert.equal(result.status, 0, result.stderr);
    const calls = f.calls()["restore-docker"];
    const removed = calls.indexOf("rm");
    assert.deepEqual(calls.slice(removed, removed + 3), ["rm", "-f", leftover]);
    assert.ok(removed < calls.indexOf("create"));
    assert.ok(calls.includes("io.chariox.chromium-restore=owned-checkpoint"));
    assert.ok(calls.includes("io.chariox.chromium-slice=fixture"));
  } finally { f.cleanup(); }
});

for (const metadata of ["other-checkpoint\nfixture\nvolume:fixture-home", "owned-checkpoint\nother-slice\nvolume:fixture-home", "owned-checkpoint\nfixture\nvolume:unrelated-home"]) {
  test(`retry preserves helper with mismatched ownership: ${metadata.replaceAll("\n", "/")}`, () => {
    const f = fixture();
    try {
      assert.equal(captureRestore(f, "none", helperId, { FIXTURE_LEFTOVER: "b".repeat(64), FIXTURE_METADATA: metadata }).status, 1);
      const calls = f.calls()["restore-docker"];
      assert.ok(!calls.includes("rm"));
      assert.ok(!calls.includes("create"));
    } finally { f.cleanup(); }
  });
}

test("a running unrelated home holder blocks restoration before file mutation", () => {
  const f = fixture();
  try {
    assert.equal(captureRestore(f, "none", helperId, { FIXTURE_HOLDER: "c".repeat(64) }).status, 1);
    const calls = f.calls()["restore-docker"];
    assert.ok(!calls.includes("rm"));
    assert.ok(!calls.includes("create"));
  } finally { f.cleanup(); }
});

test("ordinary restoration supports an empty helper-label list on the supported host Bash", () => {
  const f = fixture();
  try {
    const result = captureRestore(f, "none", helperId, { FIXTURE_MIGRATION_ID: "" });
    assert.equal(result.status, 0, result.stderr);
    const calls = f.calls()["restore-docker"];
    assert.ok(!calls.includes("ps"));
    assert.ok(!calls.includes("--label"));
    assert.deepEqual(calls.slice(-3), ["rm", "-f", helperId]);
  } finally { f.cleanup(); }
});

test("failed helper cleanup cannot report restoration success", () => {
  const f = fixture();
  try {
    assert.equal(captureRestore(f, "rm").status, 1);
    assert.deepEqual(f.calls()["restore-docker"].slice(-3), ["rm", "-f", helperId]);
  } finally { f.cleanup(); }
});

for (const failure of ["start", "cp", "exec"]) {
  test(`home restoration cleans only its created helper after ${failure} failure`, () => {
    const f = fixture();
    try {
      assert.equal(captureRestore(f, failure).status, 1);
      const calls = f.calls()["restore-docker"];
      assert.deepEqual(calls.slice(-3), ["rm", "-f", helperId]);
      assert.equal(calls.filter(arg => arg === "rm").length, 1);
    } finally { f.cleanup(); }
  });
}

test("failed helper creation never removes a pre-existing name", () => {
  const f = fixture();
  try {
    assert.equal(captureRestore(f, "create").status, 1);
    assert.ok(!f.calls()["restore-docker"].includes("rm"));
  } finally { f.cleanup(); }
});

test("a malformed creation response never authorizes removal by name", () => {
  const f = fixture();
  try {
    assert.equal(captureRestore(f, "none", "unrelated-container").status, 1);
    assert.ok(!f.calls()["restore-docker"].includes("rm"));
  } finally { f.cleanup(); }
});

test("captured restoration command preserves its quoting and scoped comparison", () => {
  const f = fixture();
  try {
    const result = captureRestore(f);
    assert.equal(result.status, 0, result.stderr);
    const body = readFileSync(join(f.capture, "restore-body"), "utf8");
    const parsed = spawnSync("bash", ["-n"], { input: body, encoding: "utf8", timeout: 1000 });
    assert.equal(parsed.status, 0, parsed.stderr);
    assert.match(body, /awk "\\\$0 ==/);
    assert.match(body, /--compare --file \/tmp\/home\.tar\.zst --verbatim-files-from/);
    assert.match(body, /\.config\/chariox-slice-chromium/);
    assert.match(body, /\.local\/share\/keyrings/);
    assert.deepEqual(f.calls()["restore-docker"].slice(-3), ["rm", "-f", helperId]);
  } finally { f.cleanup(); }
});

const gnuTar = spawnSync("tar", ["--version"], { encoding: "utf8", timeout: 1000 }).stdout?.includes("GNU tar");
test("GNU tar verifies restored browser bytes and detects a deliberately corrupted profile", { skip: !gnuTar }, () => {
  const f = fixture();
  try {
    assert.equal(captureRestore(f).status, 0);
    const source = join(f.root, "archive-source");
    const target = join(f.root, "restored-home");
    mkdirSync(join(source, ".config/chariox-slice-chromium/Default"), { recursive: true });
    mkdirSync(join(source, "profiles/[custom work]/Default"), { recursive: true });
    mkdirSync(join(source, ".local/share/keyrings"), { recursive: true });
    mkdirSync(target);
    writeFileSync(join(source, ".config/chariox-slice-chromium/Default/Cookies"), "cookie fixture");
    writeFileSync(join(source, "profiles/[custom work]/Default/Cookies"), "custom cookie fixture");
    writeFileSync(join(source, ".local/share/keyrings/login.keyring"), "keyring fixture");
    const archive = join(f.root, "real.tar.zst");
    const packed = spawnSync("tar", ["--zstd", "-cf", archive, "-C", source, "."], { encoding: "utf8", timeout: 3000 });
    assert.equal(packed.status, 0, packed.stderr);
    // Substitute only the container's fixed mount/tmp paths; execute the
    // captured production restore and compare commands with the real GNU tar.
    const body = readFileSync(join(f.capture, "restore-body"), "utf8")
      .replaceAll("/home-dst", target)
      .replaceAll("/tmp/home.tar.zst", archive)
      .replaceAll("/tmp/chariox-profile-", `${f.root}/chariox-profile-`)
      .replace("chown -R slice:slice", "true");
    const run = (script, profile = "/home/slice/.config/chariox-slice-chromium") => spawnSync("bash", ["-c", script], { env: { ...process.env, CHARIOX_VERIFY_RESTORED_PROFILE: "1", CHARIOX_SLICE_CHROME_PROFILE: profile }, encoding: "utf8", timeout: 3000 });
    const restored = run(body);
    assert.equal(restored.status, 0, restored.stderr);
    assert.ok(existsSync(join(target, ".config/chariox-slice-chromium/Default/Cookies")));
    const corrupted = run(body.replace("if [[ -n", `printf 'corrupt' > '${target}/.config/chariox-slice-chromium/Default/Cookies'\nif [[ -n`));
    assert.equal(corrupted.status, 1);
    assert.match(corrupted.stderr, /restored Chromium profile differs/);
    const customProfile = "/home/slice/profiles/[custom work]";
    const customRestored = run(body, customProfile);
    assert.equal(customRestored.status, 0, customRestored.stderr);
    const customCorrupted = run(body.replace("if [[ -n", `printf 'corrupt' > '${target}/profiles/[custom work]/Default/Cookies'\nif [[ -n`), customProfile);
    assert.equal(customCorrupted.status, 1);
    assert.match(customCorrupted.stderr, /restored Chromium profile differs/);
  } finally { f.cleanup(); }
});
