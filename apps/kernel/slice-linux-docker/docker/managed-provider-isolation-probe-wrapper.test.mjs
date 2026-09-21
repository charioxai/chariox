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

async function makeFixture(
  root,
  {
    deniedPath = "",
    deniedPathMode = 0o755,
    seedDeniedPayload = false,
    seedMaskedPayload = false,
    payloadMarker = "fixture-protected-payload-marker",
  } = {},
) {
  const home = path.join(root, "home/chariox");
  const account = path.join(home, ".provider-account/default");
  const workspace = await mkdtemp(path.join("/dev/shm", "chariox-wrapper-workspace-"));
  const maskedSliceState = path.join(root, "home/slice/.chariox");
  const deniedRoot = deniedPath ? path.join(root, deniedPath) : "";
  const wrapper = path.join(root, "wrapper.sh");
  const result = path.join(root, "wrapper-result");

  await mkdir(account, { recursive: true });
  await mkdir(maskedSliceState, { recursive: true });
  if (seedMaskedPayload) {
    await writeFile(path.join(maskedSliceState, ".protected-payload"), `${payloadMarker}\n`);
  }
  if (deniedRoot) {
    await mkdir(deniedRoot, { recursive: true });
    if (seedDeniedPayload) {
      await writeFile(path.join(deniedRoot, ".protected-payload"), `${payloadMarker}\n`);
    }
    await chmod(deniedRoot, deniedPathMode);
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
      seedMaskedPayload: true,
      payloadMarker: "masked-fixture-secret-marker",
    });
    fixtures.push(seededPayload);
    const payloadRun = await runFixture(seededPayload);

    assert.equal(payloadRun.status, 1, payloadRun.stderr || payloadRun.result);
    assert.match(payloadRun.result, /reason=denied host path contains payload in masked directory/);
    assert.match(payloadRun.result, /denied_path=.*home\/slice\/\.chariox/);
    assert.match(payloadRun.result, /denied_path_class=nonempty_directory/);
    assert.match(payloadRun.result, /denied_path_permission=readable/);
    assert.match(payloadRun.result, /denied_path_entries=1/);
    assert.doesNotMatch(payloadRun.result, /masked-fixture-secret-marker/);
    assert.doesNotMatch(payloadRun.stderr, /masked-fixture-secret-marker/);

    const readableProtectedPayload = await makeFixture(
      path.join(root, "readable-protected-payload"),
      {
        deniedPath: "var/lib/chariox",
        seedDeniedPayload: true,
        payloadMarker: "denied-fixture-secret-marker",
      },
    );
    fixtures.push(readableProtectedPayload);
    const readablePayloadRun = await runFixture(readableProtectedPayload);

    assert.equal(readablePayloadRun.status, 1, readablePayloadRun.stderr || readablePayloadRun.result);
    assert.match(readablePayloadRun.result, /denied_path=.*var\/lib\/chariox/);
    assert.match(readablePayloadRun.result, /denied_path_class=nonempty_directory/);
    assert.match(readablePayloadRun.result, /denied_path_permission=readable/);
    assert.match(readablePayloadRun.result, /denied_path_entries=1/);
    assert.doesNotMatch(readablePayloadRun.result, /denied-fixture-secret-marker/);
    assert.doesNotMatch(readablePayloadRun.stderr, /denied-fixture-secret-marker/);

    const inaccessibleMountpoint = await makeFixture(path.join(root, "inaccessible-mountpoint"), {
      deniedPath: "var/lib/chariox",
      deniedPathMode: 0o000,
    });
    fixtures.push(inaccessibleMountpoint);
    const inaccessibleRun = await runFixture(inaccessibleMountpoint);

    assert.equal(inaccessibleRun.status, 0, inaccessibleRun.stderr || inaccessibleRun.result);
    assert.match(inaccessibleRun.result, /masked_inaccessible_denied_paths=.*var\/lib\/chariox/);
    assert.match(inaccessibleRun.result, /masked_inaccessible_denied_path_permission=inaccessible/);
    assert.match(inaccessibleRun.result, /masked_inaccessible_denied_path_entries=unavailable/);

    const readableEmptyPath = await makeFixture(path.join(root, "readable-empty-path"), {
      deniedPath: "var/lib/chariox",
      deniedPathMode: 0o755,
    });
    fixtures.push(readableEmptyPath);
    const readableEmptyRun = await runFixture(readableEmptyPath);

    assert.equal(readableEmptyRun.status, 1, readableEmptyRun.stderr || readableEmptyRun.result);
    assert.match(readableEmptyRun.result, /denied_path=.*var\/lib\/chariox/);
    assert.match(readableEmptyRun.result, /denied_path_class=empty_directory/);
    assert.match(readableEmptyRun.result, /denied_path_permission=readable/);
    assert.match(readableEmptyRun.result, /denied_path_entries=0/);
  } finally {
    await Promise.all(
      fixtures.map((fixture) => rm(fixture.workspace, { recursive: true, force: true })),
    );
    await rm(root, { recursive: true, force: true });
  }
});
