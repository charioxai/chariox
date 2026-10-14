// MP-11: kernel-owned managed policies for the kernel's own Chromium.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { blockedUrl, managedPolicy, writePolicyLayer } from "./kernel-browser-policy.mjs";
import { policyLaunch } from "./kernel-browser-linux.mjs";

test("MP-11 the policy layer carries every compiled Chromium policy root", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-policy-"));
  try {
    const layer = await writePolicyLayer(root);
    for (const dir of ["chromium", "opt/chrome", "opt/chrome_for_testing"]) {
      assert.deepEqual(JSON.parse(await readFile(path.join(layer, dir, "policies/managed/chariox.json"), "utf8")), managedPolicy);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
  await assert.rejects(writePolicyLayer("/tmp/a,b"), /not mountable/);
});

test("MP-11 the policy keeps the kernel's own CDP and the native browser UI usable", () => {
  // DeveloperToolsAvailability 2 refuses the kernel's CDP attach to every page.
  assert.equal(managedPolicy.DeveloperToolsAvailability, undefined);
  for (const url of ["chrome://omnibox-popup.top-chrome", "chrome://tab-search.top-chrome", "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai"]) assert(managedPolicy.URLAllowlist.includes(url));
  for (const url of ["chrome://*", "devtools://*", "chrome-extension://*", "file://*", "view-source:*"]) assert(managedPolicy.URLBlocklist.includes(url));
  assert.equal(managedPolicy.DownloadRestrictions, 3);
  assert.equal(managedPolicy.IncognitoModeAvailability, 1);
  assert.match(managedPolicy.DefaultSearchProviderSearchURL, /^https:\/\/duckduckgo\.com\//);
});

test("MP-11 Chromium runs in a private namespace with the caller's own uid", () => {
  const launch = policyLaunch("/k/policy-etc", "/opt/chromium/chrome", ["--remote-debugging-pipe", "about:blank"], 1000, 1000);
  assert.equal(launch.command, "/usr/bin/unshare");
  assert.deepEqual(launch.args.slice(0, 4), ["--user", "--map-root-user", "--mount", "--"]);
  assert.deepEqual(launch.args.slice(-7), ["/k/policy-etc", "1000", "1000", "/opt/chromium/chrome", "--disable-setuid-sandbox", "--remote-debugging-pipe", "about:blank"]);
  assert.match(launch.args[6], /lowerdir=\$layer:\/etc" \/etc; exec \/usr\/bin\/unshare --user --map-user="\$uid"/);
  assert.throws(() => policyLaunch("/k", "/c", [], 0, 0), /invalid browser user/);
});

test("MP-11 blocked schemes", () => {
  for (const url of ["chrome://settings/", "devtools://devtools/x", "file:///etc/passwd", "view-source:https://a.example/", "chrome-extension://abc/x.html", "chrome-untrusted://x/"]) assert(blockedUrl(url), url);
  for (const url of ["https://example.com/", "about:blank", "", null, "not a url"]) assert(!blockedUrl(url), String(url));
});
