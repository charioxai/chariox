import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { dockerLabels, drillOwner } from "./lib/drill-docker-owner.mjs";

const sweep = fileURLToPath(new URL("./sweep-drill-containers.mjs", import.meta.url));
const started = new Date(Date.now() - 8 * 24 * 60 * 60 * 1000).toISOString();
const labelObject = owner => Object.fromEntries(dockerLabels(owner).filter(value => value !== "--label")
  .map(value => value.split("=")));

test("sweep previews and removes only the confirmed ended run in dependency order", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "drill-sweep-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fixture = join(dir, "inventory.json");
  const log = join(dir, "removals");
  const docker = join(dir, "docker");
  writeFileSync(docker, `#!/usr/bin/env node
const fs = require("node:fs");
const [kind, command, ...args] = process.argv.slice(2);
const id = args.at(-1);
const data = JSON.parse(fs.readFileSync(process.env.DRILL_SWEEP_FIXTURE))[kind];
if (command === "ls") console.log(Object.keys(data).join("\\n"));
else if (command === "inspect") console.log(JSON.stringify(args[1] === "{{json .Labels}}" ? data[id][1] : data[id]));
else if (command === "rm") fs.appendFileSync(process.env.DRILL_SWEEP_LOG, [kind, ...args].join(" ") + "\\n");
else process.exit(2);
`);
  chmodSync(docker, 0o700);

  const stale = labelObject(drillOwner("fixture", { run: "ended", started, detached: true }));
  const active = labelObject(drillOwner("fixture", { run: "active", started }));
  const detached = labelObject(drillOwner("fixture", { run: "recent", detached: true }));
  const partial = { "io.chariox.drill.run": "ended" };
  writeFileSync(fixture, JSON.stringify({
    image: { base: {}, derivative: stale },
    container: {
      "dead-id": ["dead-id", "/dead", stale, "no", {}, [], "base"],
      "live-id": ["live-id", "/live", active, "no", { network: { NetworkID: "shared-net" } },
        [{ Type: "volume", Name: "shared-volume" }], "base"],
      "recent-id": ["recent-id", "/recent", detached, "no", {}, [], "base"],
      "bare-id": ["bare-id", "/bare", null, "unless-stopped", {}, [], "base"],
      "partial-id": ["partial-id", "/partial", partial, "no", {}, [], "base"],
      "inherited-id": ["inherited-id", "/inherited", stale, "no", {}, [], "derivative"],
      "quota-id": ["quota-id", "/quota", {...stale, "io.chariox.slice.disk-quota":"xfs-project-v1"}, "no", {}, [], "base"],
    },
    network: {
      "dead-net": ["dead-net", "dead-network", stale],
      "shared-net": ["shared-net", "shared-network", stale],
    },
    volume: {
      "dead-volume": ["dead-volume", stale],
      "shared-volume": ["shared-volume", stale],
    },
  }));

  const env = { ...process.env, PATH: dir + ":" + process.env.PATH, DRILL_SWEEP_LOG: log, DRILL_SWEEP_FIXTURE: fixture };
  const run = (...args) => execFileSync(process.execPath, [sweep, ...args], { env, encoding: "utf8" });
  const audit = run("--dry-run", "--ended-run", "ended", "--max-age-hours", "1");
  assert.match(audit, /REPORT\tcontainer\tbare-id\t\/bare\tunlabeled, restart=unless-stopped/);
  assert.match(run("--dry-run", "--run", "ended"), /WOULD REMOVE\tcontainer\tdead-id.*age limit exceeded/);
  assert.match(audit, /REPORT\tcontainer\tpartial-id.*invalid drill ownership/);
  if (process.platform === "linux") assert.match(audit, /KEEP\tcontainer\tlive-id.*owner alive/);
  assert.match(audit, /KEEP\tcontainer\trecent-id.*within age limit/);
  const preview = run("--dry-run", "--run", "ended", "--ended-run", "ended");
  assert.match(preview, /REPORT\tcontainer\tinherited-id.*inherited from or unverifiable/);
  assert.match(preview, /REPORT\tcontainer\tquota-id.*remove quota-managed slices through their owning kernel/);
  assert.match(preview, /KEEP\tnetwork\tshared-net.*used by a retained container/);
  assert.match(preview, /KEEP\tvolume\tshared-volume.*used by a retained container/);
  assert.throws(() => readFileSync(log), { code: "ENOENT" });

  const apply = run("--apply", "--run", "ended", "--ended-run", "ended");
  assert.deepEqual(apply.split("\n").filter(line => line.startsWith("REMOVED")),
    preview.split("\n").filter(line => line.startsWith("WOULD REMOVE")).map(line => line.replace("WOULD REMOVE", "REMOVED")));
  assert.equal(readFileSync(log, "utf8"), "container --force --volumes dead-id\nnetwork dead-net\nvolume dead-volume\n");
  assert.equal(spawnSync(process.execPath, [sweep, "--check"], { env }).status, 1);
  assert.equal(spawnSync(process.execPath, [sweep, "--apply"], { env }).status, 1);
});
