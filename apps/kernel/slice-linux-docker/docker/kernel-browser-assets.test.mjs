// MP-08 / MP-10 / MP-11: native bundle must load the shared protection modules.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

test("MP-08 / MP-10: materialized native controller imports without a checkout", async () => {
  const manifest = fileURLToPath(new URL("../../src/runtime/kernel_browser_assets.rs", import.meta.url));
  const source = await readFile(manifest, "utf8");
  const root = await mkdtemp(path.join(tmpdir(), "appsbudget-assets-"));
  try {
    const entries = [...source.matchAll(/\(\s*"([^"]+)"\s*,\s*include_bytes!\("([^"]+)"\)\s*,?\s*\)/g)];
    assert.ok(entries.length > 0);
    for (const [, name, relative] of entries)
      await writeFile(path.join(root, name), await readFile(path.resolve(path.dirname(manifest), relative)));
    const child = spawnSync(process.execPath, ["--input-type=module", "-e",
      "const { pathToFileURL } = await import('node:url'); await import(pathToFileURL(process.argv[1]).href);",
      path.join(root, "kernel-browser-host.mjs")],
    { timeout: 30_000, encoding: "utf8", env: { PATH: "/usr/bin:/bin", HOME: root, TMPDIR: root } });
    assert.equal(child.status, 0, child.stderr);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
