import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

test("slice image builds use BuildKit with the normalized server architecture", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-slice-target-arch-"))
  try {
    const dockerLog = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "info") {
  if (args.includes("--format")) console.log("aarch64");
  process.exit(0);
}
if (args[0] === "buildx" && args[1] === "version") process.exit(1);
if (args[0] === "build") {
  appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify({ source: "docker", args }) + "\\n");
  process.exit(1);
}
if (args[0] === "image" && args[1] === "inspect") process.exit(1);
throw new Error("unexpected Docker operation: " + JSON.stringify(args));
`, { mode: 0o700 })
    await writeFile(join(root, "docker-buildx"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify({ source: "standalone", args }) + "\\n");
process.exit(1);
`, { mode: 0o700 })
    const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
      !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "provision"], {
      encoding: "utf8",
      timeout: 60_000,
      env: {
        ...environment,
        PATH: `${root}:${environment.PATH}`,
        TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: dockerLog,
        CHARIOX_SLICE_NAME: "chariox-target-arch-fixture",
        CHARIOX_SLICE_BUILD_IMAGE: "always",
      },
    })
    assert.notEqual(result.status, 0, "the stubbed image build should stop provisioning")
    const [build] = (await readFile(dockerLog, "utf8")).trim().split("\n").map(JSON.parse)
    const targetArg = build.args.find((value, index) => build.args[index - 1] === "--build-arg" && value.startsWith("TARGETARCH="))
    assert.equal(build.source, "standalone")
    assert.deepEqual(build.args.slice(0, 2), ["build", "--load"])
    assert.equal(targetArg, "TARGETARCH=arm64")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
