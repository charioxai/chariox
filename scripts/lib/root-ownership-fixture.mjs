import { spawnSync } from "node:child_process"

// Namespace root owns the fixture files in the real filesystem, without sudo
// or host privileges. A PID namespace also contains deliberately detached helpers.
export function runRootOwnershipFixture(testFile, testNamePattern) {
  const testArgs = ["--test", `--test-timeout=${testNamePattern ? 120_000 : 300_000}`]
  if (testNamePattern) testArgs.push(`--test-name-pattern=${testNamePattern}`)
  return spawnSync("bwrap", [
    "--unshare-user", "--unshare-pid", "--die-with-parent",
    "--uid", "0", "--gid", "0",
    "--bind", "/", "/", "--dev-bind", "/dev", "/dev", "--proc", "/proc",
    "--tmpfs", "/tmp", "--setenv", "TMPDIR", "/tmp",
    "--", process.execPath, ...testArgs, testFile,
  ], {
    encoding: "utf8", maxBuffer: 16 * 1024 * 1024, timeout: 300_000,
    env: { ...process.env, NODE_TEST_CONTEXT: undefined },
  })
}
