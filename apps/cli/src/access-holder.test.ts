// MP-08 / MP-10 / MP-11: shell depth must not change the grant's holder.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { test } from "node:test"
import { defaultAccessHolder } from "./access-holder.js"

test("the bare CLI walks past nested shells to the configured native harness", async () => {
  const cli = `import { defaultAccessHolder } from ${JSON.stringify(new URL("./access-holder.js", import.meta.url).href)}; console.log(defaultAccessHolder())`
  const env: NodeJS.ProcessEnv = { ...process.env, CHARIOX_OPENCODE_BIN: process.execPath }
  delete env.CHARIOX_CLI_ACCESS_HOLDER_PID
  const harness = spawn(process.execPath, ["-e", `
    const { spawnSync } = require("node:child_process");
    const result = spawnSync("/bin/bash", ["-c", '"$@"; exit $?', "outer", "/bin/bash", "-c", '"$@"; exit $?', "inner", process.execPath, "--input-type=module", "-e", ${JSON.stringify(cli)}], { stdio: "inherit" });
    process.exitCode = result.status ?? 1;
  `], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let output = "", errors = ""
  harness.stdout.on("data", data => { output += data })
  harness.stderr.on("data", data => { errors += data })
  const code = await new Promise<number | null>((resolve, reject) => { harness.once("exit", resolve); harness.once("error", reject) })
  assert.equal(code, 0, errors)
  assert.equal(Number(output.trim()), harness.pid)
})

test("the native launcher's captured holder remains authoritative", () => {
  const previous = process.env.CHARIOX_CLI_ACCESS_HOLDER_PID
  process.env.CHARIOX_CLI_ACCESS_HOLDER_PID = "4242"
  try { assert.equal(defaultAccessHolder(), 4242) }
  finally { if (previous === undefined) delete process.env.CHARIOX_CLI_ACCESS_HOLDER_PID; else process.env.CHARIOX_CLI_ACCESS_HOLDER_PID = previous }
})
