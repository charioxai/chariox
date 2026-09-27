import { createHash } from "node:crypto";
import { readdir, readFile, realpath, stat } from "node:fs/promises";
import path from "node:path";

const DEFAULT_PROC_ROOT = "/proc";
const BROWSER_EXECUTABLES = new Set([
  "chromium",
  "chromium-browser",
  "google-chrome",
  "google-chrome-stable",
  "chrome",
]);

const defaultFileSystem = { readdir, readFile, realpath, stat };

export class BrowserResourceInventoryError extends Error {
  constructor(message) {
    super(message);
    this.name = "BrowserResourceInventoryError";
    this.code = "browser_resource_inventory_invalid";
  }
}

/**
 * Observe the browser processes and profile directories owned by this worker.
 * The controller runs in the slice namespace, so /proc is scoped to the
 * worker rather than to the home kernel. Profile paths are reduced to opaque
 * identities before crossing the controller boundary.
 */
export async function observeBrowserResources({
  procRoot = DEFAULT_PROC_ROOT,
  fileSystem = defaultFileSystem,
} = {}) {
  const fs = { ...defaultFileSystem, ...fileSystem };
  let entries;
  try {
    entries = await fs.readdir(procRoot);
  } catch (error) {
    throw new BrowserResourceInventoryError(
      `worker browser process inventory could not be read: ${error?.message ?? String(error)}`,
    );
  }
  const observations = [];
  for (const entry of entries) {
    if (!/^\d+$/.test(String(entry))) continue;
    const pid = Number(entry);
    let args;
    try {
      const commandLine = await fs.readFile(path.join(procRoot, String(entry), "cmdline"), "utf8");
      args = String(commandLine).split("\0").filter(Boolean);
    } catch (error) {
      if (error?.code === "ENOENT" || error?.code === "ESRCH") continue;
      throw new BrowserResourceInventoryError(
        `worker browser process ${entry} could not be observed: ${error?.message ?? String(error)}`,
      );
    }
    if (!isBrowserMainProcess(args)) continue;
    const profilePath = profilePathFromArguments(args);
    if (!profilePath) {
      throw new BrowserResourceInventoryError(
        `worker browser process ${entry} omitted --user-data-dir`,
      );
    }
    let canonicalProfilePath;
    let profileStats;
    try {
      canonicalProfilePath = await fs.realpath(profilePath);
      profileStats = await fs.stat(canonicalProfilePath);
    } catch (error) {
      throw new BrowserResourceInventoryError(
        `worker browser profile for process ${entry} could not be observed: ${error?.message ?? String(error)}`,
      );
    }
    if (typeof profileStats?.isDirectory !== "function" || !profileStats.isDirectory()) {
      throw new BrowserResourceInventoryError(
        `worker browser profile for process ${entry} is not a directory`,
      );
    }
    observations.push({
      pid,
      profilePath: canonicalProfilePath,
      device: profileStats.dev,
      inode: profileStats.ino,
    });
  }

  if (observations.length === 0) {
    throw new BrowserResourceInventoryError(
      "worker browser resource inventory found no Chromium main process",
    );
  }

  return {
    browser_ids: observations
      .sort((left, right) => left.pid - right.pid)
      .map(({ pid }) => `browser-pid-${pid}`),
    profile_ids: [...new Map(
      observations.map((observation) => [
        profileIdentitySource(observation),
        `profile-sha256-${hashProfileIdentity(observation)}`,
      ]),
    ).values()].sort(),
  };
}

export function validateBrowserResourceInventory(value) {
  if (!value || typeof value !== "object") {
    throw new BrowserResourceInventoryError("worker browser resource inventory must be an object");
  }
  const browserIds = identityArray(value.browser_ids, "browser_ids");
  const profileIds = identityArray(value.profile_ids, "profile_ids");
  if (browserIds.length !== 1 || profileIds.length !== 1) {
    throw new BrowserResourceInventoryError(
      `worker browser resource inventory must observe exactly one browser and one profile (got ${browserIds.length} browsers and ${profileIds.length} profiles)`,
    );
  }
  return { browser_ids: browserIds, profile_ids: profileIds };
}

function identityArray(value, label) {
  if (!Array.isArray(value)) {
    throw new BrowserResourceInventoryError(`worker browser resource inventory ${label} must be an array`);
  }
  const identities = value.map((identity) => {
    if (typeof identity !== "string" || identity.trim() === "") {
      throw new BrowserResourceInventoryError(
        `worker browser resource inventory ${label} contains an invalid identity`,
      );
    }
    return identity.trim();
  });
  if (new Set(identities).size !== identities.length) {
    throw new BrowserResourceInventoryError(
      `worker browser resource inventory ${label} contains duplicate identities`,
    );
  }
  return identities;
}

function isBrowserMainProcess(args) {
  const executable = path.basename(args[0] ?? "").toLowerCase();
  return BROWSER_EXECUTABLES.has(executable) && !args.some((arg) => arg.startsWith("--type="));
}

function profilePathFromArguments(args) {
  const inline = args.find((arg) => arg.startsWith("--user-data-dir="));
  if (inline) return inline.slice("--user-data-dir=".length);
  const index = args.indexOf("--user-data-dir");
  return index >= 0 ? args[index + 1] : null;
}

function profileIdentitySource({ profilePath, device, inode }) {
  return `${profilePath}\0${String(device ?? "")}\0${String(inode ?? "")}`;
}

function hashProfileIdentity(observation) {
  return createHash("sha256")
    .update(profileIdentitySource(observation))
    .digest("hex");
}
