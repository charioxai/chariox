import assert from "node:assert/strict"

export async function verifyDirectDockerEngineAccess({ target, writable, platform, imageId, runCommand }) {
  assert.ok(typeof target === "string" && target.startsWith("/"), "direct-Docker fixture path must be absolute")
  assert.equal(typeof runCommand, "function", "direct-Docker command runner is required")
  if (platform === "linux") {
    for (const flag of writable ? ["-x", "-w"] : ["-x"]) {
      const result = await runCommand("runuser", ["-u", "chariox-docker", "--", "test", flag, target], 10_000)
      if (result.code !== 0) throw new Error("rootless Docker engine user cannot access the selected fixture root")
    }
    return
  }
  if (platform !== "darwin") throw new Error(`direct-Docker fixture access is unsupported on ${platform}`)
  assert.ok(typeof imageId === "string" && imageId.startsWith("sha256:"),
    "macOS direct-Docker drill requires an exact prebuilt slice image")
  const mount = `type=bind,source=${target},target=/chariox-drill-access${writable ? "" : ",readonly"}`
  const result = await runCommand("docker", [
    "run", "--rm", "--pull", "never", "--network", "none", "--read-only",
    "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
    "--memory", "128m", "--cpus", "0.25", "--pids-limit", "32",
    "--user", "slice", "--entrypoint", "/bin/sh", "--mount", mount,
    imageId, "-c", `test -x /chariox-drill-access${writable ? " && test -w /chariox-drill-access" : ""}`,
  ], 30_000)
  if (result.code !== 0) {
    throw new Error(`local Docker engine cannot ${writable ? "write and traverse" : "traverse"} the selected fixture path: ${result.stderr.trim()}`)
  }
}
