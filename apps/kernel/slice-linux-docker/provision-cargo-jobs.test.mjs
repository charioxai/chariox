import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

// The engine reports `cpus memory` for the NCPU/MemTotal query; every build is refused so the arguments can be read.
async function buildArguments(engine, extraEnvironment = {}) {
  const root = await mkdtemp(join(tmpdir(), "chariox-slice-cargo-jobs-"))
  try {
    const log = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "info") { console.log(args.join(" ").includes("NCPU") ? ${JSON.stringify(engine)} : "x86_64"); process.exit(0); }
if (args[0] === "buildx" && args[1] === "version") process.exit(0);
if (args[0] === "buildx") { appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n"); process.exit(1); }
if (args[0] === "image" && args[1] === "inspect") process.exit(1);
throw new Error("unexpected Docker operation: " + JSON.stringify(args));
`, { mode: 0o700 })
    const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "build-image"], {
      encoding: "utf8", timeout: 60_000,
      env: { ...environment, PATH: `${root}:${environment.PATH}`, TMPDIR: root, CHARIOX_TEST_DOCKER_LOG: log,
        CHARIOX_SLICE_NAME: "chariox-cargo-jobs-fixture", CHARIOX_SLICE_BUILD_IMAGE: "always", ...extraEnvironment },
    })
    const [build] = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse)
    return { build, stderr: result.stderr }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}
const jobs = ({ build }) => build.find(value => value.startsWith("CHARIOX_CARGO_BUILD_JOBS="))

test("a source build uses the engine's CPUs, three GiB of memory per job, at most four jobs", async () => {
  assert.equal(jobs(await buildArguments("16 68719476736")), "CHARIOX_CARGO_BUILD_JOBS=4")
  assert.equal(jobs(await buildArguments("2 68719476736")), "CHARIOX_CARGO_BUILD_JOBS=2")
  assert.equal(jobs(await buildArguments("16 8589934592")), "CHARIOX_CARGO_BUILD_JOBS=2")
  assert.equal(jobs(await buildArguments("8 4294967296")), "CHARIOX_CARGO_BUILD_JOBS=1")
})

test("an engine that does not report CPUs and memory builds with one job, and the override wins", async () => {
  assert.equal(jobs(await buildArguments("unknown")), "CHARIOX_CARGO_BUILD_JOBS=1")
  assert.equal(jobs(await buildArguments("16 68719476736", { CHARIOX_SLICE_CARGO_BUILD_JOBS: "3" })), "CHARIOX_CARGO_BUILD_JOBS=3")
})
