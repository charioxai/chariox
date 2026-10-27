import test from "node:test"
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

const moduleUrl = new URL("../apps/kernel/slice-linux-docker/local-docker-broker-lifetime.mjs", import.meta.url).href

for (const trigger of ["SIGTERM", "SIGINT", "SIGHUP", "parent-exit"]) {
  test(`broker cleans up during unfinished startup after ${trigger}`, {timeout: 10_000}, async () => {
    const directory = await mkdtemp(join(tmpdir(), "chariox-broker-lifetime-"))
    const parent = join(directory, "parent-alive")
    const result = join(directory, "cleanup")
    await writeFile(parent, "synthetic parent lifetime")
    const child = spawn(process.execPath, ["--input-type=module", "-e", `
      import { watchLocalBrokerLifetime } from ${JSON.stringify(moduleUrl)};
      import { appendFileSync, existsSync } from "node:fs";
      watchLocalBrokerLifetime(() => appendFileSync(${JSON.stringify(result)}, "stopped\\n"),
        () => existsSync(${JSON.stringify(parent)}));
      process.stdout.write("starting\\n");
      await new Promise(() => {});
    `], {stdio: ["ignore", "pipe", "pipe"]})
    const exited = once(child, "exit")
    try {
      await once(child.stdout, "data")
      if (trigger === "parent-exit") await rm(parent)
      else child.kill(trigger)
      assert.deepEqual(await exited, [0, null])
      assert.equal(await readFile(result, "utf8"), "stopped\n")
    } finally {
      if (child.exitCode === null && child.signalCode === null) { child.kill("SIGKILL"); await exited }
      await rm(directory, {recursive: true})
    }
  })
}
