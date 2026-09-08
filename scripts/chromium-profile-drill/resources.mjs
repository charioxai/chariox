import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, realpathSync } from "node:fs";
import { basename, dirname, join } from "node:path";

export function checked(command, args, { timeout = 30000, env = process.env } = {}) {
  // GNU timeout owns the command group throughout execution. Never send a
  // signal to a PID/PGID after spawnSync has returned and reaped its leader.
  const result = spawnSync("timeout", ["--kill-after=2s", `${Math.ceil(timeout / 1000)}s`, command, ...args], {
    encoding: "utf8", env, maxBuffer: 1024 * 1024,
  });
  if (result.error || result.status !== 0) throw new Error(`${command} failed (${result.status}): ${(result.stderr ?? "").slice(-8192)}`);
  return result.stdout.trim();
}
export const docker = (args, options) => checked("docker", args, options);

export function loadOwner(scratch) {
  assert.equal(process.env.GITHUB_ACTIONS, "true", "hosted drill only");
  scratch = realpathSync(scratch);
  assert.equal(dirname(scratch), realpathSync(process.env.RUNNER_TEMP));
  assert.match(basename(scratch), /^chariox-chromium\.[A-Za-z0-9]+$/);
  const bytes = readFileSync(join(scratch, "owner.json"));
  assert.ok(bytes.length < 16384);
  const owner = JSON.parse(bytes);
  assert.match(owner.id, /^[a-f0-9]{24}$/);
  assert.equal(owner.image, `chariox-chromium-drill:${owner.id}`);
  return owner;
}

export function cleanup(scratch) {
  cleanupResources(loadOwner(scratch));
}

export function cleanupResources(owner, command = docker) {
  // Labels identify resources even when creation succeeded but its response
  // was lost. Inspect again and remove containers/images by immutable ID.
  const label = `io.chariox.chromium-drill=${owner.id}`;
  const containers = command(["ps", "-aq", "--no-trunc", "--filter", `label=${label}`]).split(/\s+/).filter(Boolean);
  assert.ok(containers.length <= 16);
  for (const id of containers) {
    assert.match(id, /^[a-f0-9]{64}$/);
    assert.equal(command(["inspect", "--format", '{{index .Config.Labels "io.chariox.chromium-drill"}}', id]), owner.id);
    command(["rm", "--force", id]);
  }
  const volumes = command(["volume", "ls", "-q", "--filter", `label=${label}`]).split(/\s+/).filter(Boolean);
  assert.ok(volumes.length <= 4);
  for (const volume of volumes) {
    assert.match(volume, new RegExp(`^chariox-chromium-${owner.id}-(source|restored|empty)$`));
    assert.equal(command(["volume", "inspect", "--format", '{{index .Labels "io.chariox.chromium-drill"}}', volume]), owner.id);
    command(["volume", "rm", volume]);
  }
  const images = command(["image", "ls", "-q", "--no-trunc", "--filter", `label=${label}`]).split(/\s+/).filter(Boolean);
  assert.ok(images.length <= 4);
  for (const id of new Set(images)) {
    assert.match(id, /^sha256:[a-f0-9]{64}$/);
    assert.equal(command(["image", "inspect", "--format", '{{index .Config.Labels "io.chariox.chromium-drill"}}', id]), owner.id);
    command(["image", "rm", id]);
  }
}
