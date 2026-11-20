import { randomUUID } from "node:crypto";
import { readFileSync, readlinkSync } from "node:fs";
import { basename } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const labelKeys = {
  run: "io.chariox.drill.run",
  started: "io.chariox.drill.started",
  harness: "io.chariox.drill.harness",
  process: "io.chariox.drill.process",
};

export const runPattern = /^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,179}$/;

export function processScope() {
  if (process.platform !== "linux") return null;
  return readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim() + "/" +
    readlinkSync("/proc/self/ns/pid").slice(5, -1);
}

export function processStart(pid) {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
    const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
    return ["Z", "X"].includes(fields[0]) ? null : fields[19];
  } catch (error) {
    if (error.code === "ENOENT" || error.code === "ESRCH") return null;
    throw error;
  }
}

export function drillOwner(harness, { run = process.env.CHARIOX_DRILL_RUN || randomUUID(),
  started = process.env.CHARIOX_DRILL_STARTED || new Date().toISOString(),
  pid = process.pid, detached = false } = {}) {
  const scope = detached ? null : processScope();
  const start = scope && processStart(pid);
  if (scope && !start) throw new Error(`Drill owner process ${pid} has ended`);
  const owner = { run, started, harness: harness.startsWith("file:") ? basename(fileURLToPath(harness)) : harness,
    process: scope ? `${scope}/${pid}/${start}` : "" };
  if (!validOwner(owner)) throw new Error("Invalid drill run, start time or harness");
  return owner;
}

export function validOwner({ run, started, harness, process: identity }) {
  return [run, started, harness, identity ?? ""].every(value => typeof value === "string") &&
    runPattern.test(run) && runPattern.test(harness) &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?Z$/.test(started) &&
    Number.isFinite(Date.parse(started)) &&
    (!identity || /^[a-f0-9-]{36}\/\d+\/[1-9]\d*\/\d+$/.test(identity));
}

export function dockerLabels(owner) {
  return Object.entries(labelKeys).flatMap(([field, key]) =>
    owner[field] ? ["--label", `${key}=${owner[field]}`] : []);
}

export function drillDockerLabels(harness, options) {
  return dockerLabels(drillOwner(harness, options));
}

export function drillEnvironment(owner) {
  return Object.fromEntries(Object.keys(labelKeys).map(field =>
    [`CHARIOX_DRILL_${field.toUpperCase()}`, owner[field]]));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, harness, pid] = process.argv.slice(2);
  if (!["labels", "environment", "detached"].includes(mode) || !harness) {
    throw new Error("Usage: drill-docker-owner.mjs labels|environment|detached HARNESS [OWNER_PID]");
  }
  const owner = drillOwner(harness, { detached: mode === "detached", pid: Number(pid || process.ppid) });
  const lines = mode === "labels" ? dockerLabels(owner) :
    Object.entries(drillEnvironment(owner)).map(([key, value]) => `${key}=${value}`);
  process.stdout.write(lines.join("\n") + "\n");
}
