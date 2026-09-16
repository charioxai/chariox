import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const wrapperSourcePath = fileURLToPath(
  new URL("./managed-provider-isolation-probe-wrapper.sh", import.meta.url),
);

function renderWrapper(source, root) {
  const procRootPath = "__CHARIOX_WRAPPER_TEST_PROC_ROOT__";
  return source
    .replaceAll("/proc/1/root/var/lib/chariox", procRootPath)
    .replaceAll("/home/slice/.chariox", path.join(root, "home/slice/.chariox"))
    .replaceAll(
      "/run/chariox-slice-broker.sock",
      path.join(root, "run/chariox-slice-broker.sock"),
    )
    .replaceAll("/var/lib/chariox", path.join(root, "var/lib/chariox"))
    .replaceAll("/home/chariox", path.join(root, "home/chariox"))
    .replaceAll(procRootPath, path.join(root, "proc-root/var/lib/chariox"));
}

async function makeFixture(root, { seededPayload = false, visibleDeniedPath = "" } = {}) {
  const home = path.join(root, "home/chariox");
  const account = path.join(home, ".provider-account/default");
  const workspace = await mkdtemp(path.join("/dev/shm", "chariox-wrapper-workspace-"));
  const maskedSliceState = path.join(root, "home/slice/.chariox");
  const wrapper = path.join(root, "wrapper.sh");
  const result = path.join(root, "wrapper-result");

  await mkdir(account, { recursive: true });
  await mkdir(maskedSliceState, { recursive: true });
  if (seededPayload) {
    await writeFile(path.join(maskedSliceState, ".protected-payload"), "must-not-be-visible\n");
  }
  if (visibleDeniedPath) {
    await mkdir(path.join(root, visibleDeniedPath), { recursive: true });
  }

  const source = await readFile(wrapperSourcePath, "utf8");
  await writeFile(wrapper, renderWrapper(source, root));
  await chmod(wrapper, 0o755);

  return {
    account,
    result,
    wrapper,
    workspace,
    env: {
      PATH: "/usr/bin:/bin",
      HOME: home,
      CODEX_HOME: account,
      CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE: workspace,
      CHARIOX_MANAGED_ISOLATION_PROBE_RESULT: result,
      CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER: "/bin/true",
      CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE: "1",
      CHARIOX_MANAGED_ISOLATION_ASSERT_MODE: "strict",
    },
  };
}

async function runFixture(fixture) {
  try {
    const output = await execFileAsync(fixture.wrapper, [], {
      cwd: fixture.workspace,
      env: fixture.env,
      maxBuffer: 32 * 1024,
    });
    return {
      result: await readFile(fixture.result, "utf8").catch(() => ""),
      status: 0,
      stderr: output.stderr,
      stdout: output.stdout,
    };
  } catch (error) {
    return {
      result: await readFile(fixture.result, "utf8").catch(() => ""),
      status:
        error.status ??
        (typeof error.code === "number" ? error.code : -1),
      stderr: error.stderr ?? "",
      stdout: error.stdout ?? "",
    };
  }
}

test("allows only an empty masked slice state directory and diagnoses payload", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-wrapper-regression-"));
  const fixtures = [];
  try {
    const emptyMask = await makeFixture(path.join(root, "empty-mask"));
    fixtures.push(emptyMask);
    const emptyRun = await runFixture(emptyMask);

    assert.equal(emptyRun.status, 0, emptyRun.stderr || emptyRun.result);
    assert.match(emptyRun.result, /^managed_provider_isolation=ok$/m);
    assert.match(emptyRun.result, /masked_empty_denied_path=.*home\/slice\/\.chariox/);

    const seededPayload = await makeFixture(path.join(root, "seeded-payload"), {
      seededPayload: true,
    });
    fixtures.push(seededPayload);
    const payloadRun = await runFixture(seededPayload);

    assert.equal(payloadRun.status, 1, payloadRun.stderr || payloadRun.result);
    assert.match(payloadRun.result, /reason=denied host path contains payload in masked directory/);
    assert.match(payloadRun.result, /denied_path=.*home\/slice\/\.chariox/);
    assert.match(payloadRun.result, /denied_path_class=nonempty_directory/);
    assert.match(payloadRun.result, /denied_path_entries=1/);

    const ordinaryDeniedPath = await makeFixture(path.join(root, "ordinary-denied-path"), {
      visibleDeniedPath: "var/lib/chariox",
    });
    fixtures.push(ordinaryDeniedPath);
    const ordinaryDeniedRun = await runFixture(ordinaryDeniedPath);

    assert.equal(ordinaryDeniedRun.status, 1, ordinaryDeniedRun.stderr || ordinaryDeniedRun.result);
    assert.match(ordinaryDeniedRun.result, /denied_path=.*var\/lib\/chariox/);
    assert.match(ordinaryDeniedRun.result, /denied_path_class=directory/);
  } finally {
    await Promise.all(
      fixtures.map((fixture) => rm(fixture.workspace, { recursive: true, force: true })),
    );
    await rm(root, { recursive: true, force: true });
  }
});
