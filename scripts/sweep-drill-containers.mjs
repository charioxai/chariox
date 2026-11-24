#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { parseArgs } from "node:util";
import { labelKeys, processScope, processStart, runPattern, validOwner } from "./lib/drill-docker-owner.mjs";

const { values } = parseArgs({ options: {
  "dry-run": { type: "boolean" },
  apply: { type: "boolean" },
  check: { type: "boolean" },
  run: { type: "string" },
  "ended-run": { type: "string", multiple: true, default: [] },
  "max-age-hours": { type: "string", default: "168" },
  help: { type: "boolean" },
} });
if (values.help) {
  console.log("Usage: sweep-drill-containers.mjs [--dry-run | --check | --apply] [--run ID] [--ended-run ID]... [--max-age-hours 168]");
  process.exit(0);
}
const maxAge = Number(values["max-age-hours"]) * 3_600_000;
if ([values.apply, values.check, values["dry-run"]].filter(Boolean).length > 1 ||
    !Number.isFinite(maxAge) || maxAge <= 0 ||
    [values.run, ...values["ended-run"]].some(id => id !== undefined && !runPattern.test(id)) ||
    (values.apply && !values.run)) {
  throw new Error("Choose one mode, a positive age limit and valid run IDs. --apply requires --run ID.");
}

const ended = new Set(values["ended-run"]);
const now = Date.now();
const scope = processScope();
const json = (field) => `{{json ${field}}}`;
const docker = (...args) => execFileSync("docker", args, {
  encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 60_000,
}).trim();

function inventory(kind, fields) {
  const listArgs = kind === "container" ? ["--all", "--no-trunc", "--quiet"] :
    kind === "network" ? ["--no-trunc", "--quiet"] : ["--quiet"];
  if (kind !== "container") listArgs.push("--filter", `label=${labelKeys.run}`);
  const format = `[${fields.map(json).join(",")}]`;
  return docker(kind, "ls", ...listArgs).split("\n").filter(Boolean).flatMap(id => {
    try {
      return [JSON.parse(docker(kind, "inspect", "--format", format, id))];
    } catch (error) {
      if (error.status === 1 && /No such (container|network|volume)/i.test(error.stderr)) return [];
      throw error;
    }
  });
}

function decide(labels) {
  const owner = Object.fromEntries(Object.entries(labelKeys).map(([field, key]) => [field, labels?.[key]]));
  if (values.run && owner.run && values.run !== owner.run) return null;
  if (!validOwner(owner) || Date.parse(owner.started) > now) {
    return { report: owner.run ? "invalid drill ownership" : "unlabeled" };
  }
  // Docker removal alone does not release the broker's XFS quota allocation.
  const expired = reason => labels["io.chariox.slice.disk-quota"]
    ? { owner, report: `${reason}; remove quota-managed slices through their owning kernel` }
    : { owner, remove: reason };
  const [boot, namespace, pid, start] = owner.process?.split("/") ?? [];
  if (scope && `${boot}/${namespace}` === scope) {
    if (processStart(pid) === start) return { owner, keep: "owner alive" };
    return expired("owner ended");
  }
  if (ended.has(owner.run)) return expired("owner ended (confirmed)");
  if (now - Date.parse(owner.started) >= maxAge) return expired("age limit exceeded");
  return { owner, keep: "within age limit" };
}

const candidates = [];
let failedCheck = false;
function report(kind, id, name, decision, problem) {
  if (!decision) return;
  const { owner, remove, keep, report: warning } = decision;
  const details = [kind, id, name, owner?.run, owner?.started, owner?.harness].filter(Boolean);
  if (warning || problem || remove) failedCheck = true;
  if (warning || problem) console.log(["REPORT", ...details, [warning, problem].filter(Boolean).join(", ")].join("\t"));
  if (remove) candidates.push([kind, id, name, remove, owner]);
  if (keep) console.log(["KEEP", ...details, keep].join("\t"));
}

function hasLaunchOwnership(image, labels) {
  // A Docker commit copies labels into the image. Its next container may
  // inherit them even when the launcher had no connection to this drill.
  try {
    const imageLabels = JSON.parse(docker("image", "inspect", "--format", json(".Config.Labels"), image));
    return imageLabels?.[labelKeys.run] !== labels[labelKeys.run];
  } catch (error) {
    if (error.status === 1 && /no such image/i.test(error.stderr)) return false;
    throw error;
  }
}

const retainedNetworks = new Set();
const retainedVolumes = new Set();
for (const [id, name, labels, policy, networks, mounts, image] of inventory("container", [
  ".Id", ".Name", ".Config.Labels", ".HostConfig.RestartPolicy.Name", ".NetworkSettings.Networks", ".Mounts", ".Image",
])) {
  let decision = decide(labels);
  if (decision?.remove && !hasLaunchOwnership(image, labels)) {
    decision = { owner: decision.owner, report: "drill ownership is inherited from or unverifiable against the image" };
  }
  if (!decision?.remove) {
    for (const network of Object.values(networks ?? {})) retainedNetworks.add(network.NetworkID);
    for (const mount of mounts ?? []) if (mount.Type === "volume") retainedVolumes.add(mount.Name);
  }
  report("container", id, name, decision, policy && policy !== "no" ? `restart=${policy}` : "");
}
for (const [id, name, labels] of inventory("network", [".Id", ".Name", ".Labels"])) {
  const decision = decide(labels);
  if (retainedNetworks.has(id) && decision?.remove) {
    report("network", id, name, { owner: decision.owner, keep: "used by a retained container" });
  } else {
    report("network", id, name, decision);
  }
}
for (const [name, labels] of inventory("volume", [".Name", ".Labels"])) {
  const decision = decide(labels);
  if (retainedVolumes.has(name) && decision?.remove) {
    report("volume", name, "", { owner: decision.owner, keep: "used by a retained container" });
  } else {
    report("volume", name, "", decision);
  }
}

for (const [kind, id, name, reason, owner] of candidates) {
  if (values.apply) {
    // Volumes have no immutable Docker ID. Check that the exact name still belongs to this run.
    if (kind === "volume") {
      const labels = JSON.parse(docker(kind, "inspect", "--format", json(".Labels"), id));
      if (Object.entries(labelKeys).some(([field, key]) => (labels?.[key] ?? "") !== (owner[field] ?? ""))) {
        throw new Error(`Volume ownership changed before removal: ${id}`);
      }
    }
    docker(kind, "rm", ...(kind === "container" ? ["--force", "--volumes"] : []), id);
  }
  console.log([values.apply ? "REMOVED" : values.check ? "STALE" : "WOULD REMOVE",
    kind, id, name, owner.run, owner.started, owner.harness, reason].filter(Boolean).join("\t"));
}
if (values.check && failedCheck) process.exitCode = 1;
