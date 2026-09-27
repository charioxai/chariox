import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import { observeBrowserResources } from "./browser-controller-resources.mjs";

test("worker resource observation derives opaque identities from Chromium process/profile state", async () => {
  const files = new Map([
    ["/worker-proc/101/cmdline", "/usr/lib/chromium/chromium\0--user-data-dir=/home/slice/profile\0"],
    ["/worker-proc/102/cmdline", "/usr/lib/chromium/chromium\0--type=renderer\0--user-data-dir=/home/slice/profile\0"],
    ["/worker-proc/notes/cmdline", "not a process"],
  ]);
  const observed = await observeBrowserResources({
    procRoot: "/worker-proc",
    fileSystem: {
      readdir: async () => ["101", "102", "notes"],
      readFile: async (file) => files.get(file),
      realpath: async (file) => `/worker-real${file.slice("/home".length)}`,
      stat: async () => ({
        dev: 7,
        ino: 41,
        isDirectory: () => true,
      }),
    },
  });

  assert.deepEqual(observed.browser_ids, ["browser-pid-101"]);
  assert.deepEqual(observed.profile_ids, [
    `profile-sha256-${createHash("sha256")
      .update(["/worker-real/slice/profile", "7", "41"].join("\0"))
      .digest("hex")}`,
  ]);
});

test("worker resource observation fails closed when no main Chromium process is visible", async () => {
  await assert.rejects(
    observeBrowserResources({
      procRoot: "/worker-proc",
      fileSystem: {
        readdir: async () => ["101"],
        readFile: async () => "/usr/lib/chromium/chromium\0--type=renderer\0",
      },
    }),
    (error) => error.code === "browser_resource_inventory_invalid",
  );
});
