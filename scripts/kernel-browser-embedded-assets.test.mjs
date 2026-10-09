// MP-08 / MP-10 / MP-11: the native browser must boot without a source checkout.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile, mkdtemp, writeFile, rm } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const exec = promisify(execFile);
const source = fileURLToPath(new URL("../apps/kernel/src/runtime/kernel_browser_assets.rs", import.meta.url));

test("MP-08/MP-10/MP-11 embedded host imports all controller dependencies outside the checkout", async () => {
  const inventory = await readFile(source, "utf8");
  const assets = [...inventory.matchAll(/"([^"/]+\.(?:mjs|py))",\s*include_bytes!\("([^"]+)"\)/g)];
  assert(assets.length > 0, "native asset inventory must be present");
  const directory = await mkdtemp(path.join(tmpdir(), "chariox-embedded-browser-"));
  try {
    for (const [, name, relative] of assets) {
      await writeFile(path.join(directory, name), await readFile(path.resolve(path.dirname(source), relative)));
    }
    // Import the real host from only its embedded files. This catches omitted
    // transitive modules before Chrome starts, without launching a fixture browser.
    await exec(process.execPath, ["--input-type=module", "-e", "await import(process.argv[1])",
      pathToFileURL(path.join(directory, "kernel-browser-host.mjs")).href], { cwd: directory });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
