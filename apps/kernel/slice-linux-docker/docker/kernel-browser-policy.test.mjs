// MP-11: kernel-owned managed policies for the kernel's own Chromium.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { blockedUrl, extensionManagementUrl, managedPolicy, writePolicyLayer } from "./kernel-browser-policy.mjs";
import { TAB_GROUPS_EXTENSION_ID } from "./kernel-browser-tab-groups.mjs";
import { browserLauncher, candidates, namespaceLauncher, pinnedChromium, policyLaunch, policyNamespaceAvailable } from "./kernel-browser-linux.mjs";

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
  for (const url of ["chrome://*", "devtools://*", "file://*", "view-source:*"]) assert(managedPolicy.URLBlocklist.includes(url));
  assert.equal(managedPolicy.DownloadRestrictions, 3);
  // Miguel 2026-10-15: the person installs and manages extensions from the Web Store as in any Chrome
  // (popups, options pages, chrome://extensions); nothing is sideloaded. Agents cannot (host refusal).
  assert.equal(managedPolicy.ExtensionInstallBlocklist, undefined);
  assert.deepEqual(managedPolicy.ExtensionInstallSources, ["https://chromewebstore.google.com/*"]); assert.equal(managedPolicy.BlockExternalExtensions, true);
  for (const blocked of ["chromewebstore.google.com", "chrome.google.com/webstore", "chrome-extension://*"]) assert(!managedPolicy.URLBlocklist.includes(blocked), blocked);
  assert(managedPolicy.URLAllowlist.includes("chrome://extensions"));
  assert(managedPolicy.URLAllowlist.includes("chrome-extension://ecideajikgeiboaphimcgeahicjgcfea"));
  assert.equal(managedPolicy.IncognitoModeAvailability, 1);
  // A same-size fullscreen window would put page pixels in the band crop (vidlat review, 2026-10-15).
  assert.equal(managedPolicy.FullscreenAllowed, false);
  assert.deepEqual(managedPolicy.ExtensionSettings, { [TAB_GROUPS_EXTENSION_ID]: { toolbar_pin: "default_unpinned" } }, "no pinned extension icon in the band");
  assert.equal(managedPolicy.WebAppInstallByUserEnabled, false);
  assert.match(managedPolicy.DefaultSearchProviderSearchURL, /^https:\/\/duckduckgo\.com\//);
});

test("MP-11 Chromium runs in a private namespace with the caller's own uid", () => {
  const launch = policyLaunch("/k/policy-etc", "/k/private", "/opt/chromium/chrome", ["--remote-debugging-pipe", "about:blank"], 1000, 1000);
  assert.equal(launch.command, "/usr/bin/unshare");
  assert.deepEqual(launch.args.slice(0, 4), ["--user", "--map-root-user", "--mount", "--"]);
  assert.deepEqual(launch.args.slice(-9), ["/usr/bin/unshare", "/k/policy-etc", "/k/private", "1000", "1000", "/opt/chromium/chrome", "--disable-setuid-sandbox", "--remote-debugging-pipe", "about:blank"]);
  assert.match(launch.args[6], /lowerdir=\$layer:\/etc" \/etc; \/usr\/bin\/mount --bind "\$temporary" \/tmp; export TMPDIR=\/tmp; exec "\$launcher" --user --map-user="\$uid"/);
  assert.throws(() => policyLaunch("/k", "/k/private", "/c", [], 0, 0), /invalid browser user/);
});

// Real host, stock Ubuntu 24.04 (2026-10-14, 168.119.96.126): AppArmor lets the user namespace
// be created but refuses its id map, so the namespaced launch never started Chromium.
test("MP-11 a refused private namespace is detected before launch", () => {
  const calls = [];
  assert.equal(policyNamespaceAvailable((command, args) => { calls.push([command, ...args]); return { status: 1 }; }, "/usr/bin/unshare"), false);
  assert.equal(policyNamespaceAvailable(() => ({ status: 0 }), "/usr/bin/unshare"), true);
  assert.deepEqual(calls, [["/usr/bin/unshare", "--user", "--map-root-user", "--mount", "--", "/bin/true"]]);
});

// Ubuntu >= 23.10: install-root.sh installs a root-owned launcher that the AppArmor
// profile chariox-browser-unshare admits; both namespace steps exec that launcher.
test("MP-11 the Setup-installed launcher creates both namespaces when it is root-owned", () => {
  const file = (uid, mode) => () => ({ isFile: () => true, uid, mode: 0o100000 | mode });
  assert.equal(namespaceLauncher(file(0, 0o755)), browserLauncher);
  assert.equal(namespaceLauncher(file(1000, 0o755)), "/usr/bin/unshare", "a user-owned file is never trusted");
  assert.equal(namespaceLauncher(file(0, 0o777)), "/usr/bin/unshare", "a writable file is never trusted");
  assert.equal(namespaceLauncher(() => { throw Object.assign(new Error("missing"), { code: "ENOENT" }); }), "/usr/bin/unshare");
  const launch = policyLaunch("/k/policy-etc", "/k/private", "/opt/chromium/chrome", ["about:blank"], 1000, 1000, browserLauncher);
  assert.equal(launch.command, browserLauncher);
  assert.match(launch.args[6], /exec "\$launcher" --user --map-user="\$uid"/);
  assert.deepEqual(launch.args.slice(7, 9), ["chariox-browser", browserLauncher]);
  const calls = [];
  policyNamespaceAvailable((command, args) => { calls.push(command); return { status: 0 }; }, browserLauncher);
  assert.deepEqual(calls, [browserLauncher]);
});

// Automated drills use DuckDuckGo as the home page (coordinator 2026-10-14, Google is
// manual-only); the product default stays the policy's until Miguel decides.
test("MP-11 an https home page override sets the native new tab and home page", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-policy-"));
  try {
    const read = async env => JSON.parse(await readFile(path.join(await writePolicyLayer(root, env), "chromium/policies/managed/chariox.json"), "utf8"));
    const home = await read({ CHARIOX_KERNEL_BROWSER_HOME_URL: "https://duckduckgo.com/" });
    assert.equal(home.NewTabPageLocation, "https://duckduckgo.com/"); assert.equal(home.HomepageLocation, "https://duckduckgo.com/");
    for (const value of ["http://duckduckgo.com/", "chrome://settings", "https://user:pw@example.com/", "not a url"])
      assert.equal((await read({ CHARIOX_KERNEL_BROWSER_HOME_URL: value })).NewTabPageLocation, managedPolicy.NewTabPageLocation, value);
  } finally { await rm(root, { recursive: true, force: true }); }
});

// Miguel 2026-10-14: the Setup installs one pinned Chromium; the kernel prefers it.
test("MP-11 the kernel browser prefers the Setup's pinned Chromium when it is root-owned", async () => {
  const lock = JSON.parse(await readFile(new URL("../../../../deploy/local-linux/kernel-browser-chromium.lock.json", import.meta.url), "utf8"));
  assert.equal(pinnedChromium, `/usr/lib/chariox/browser/chromium/${lock.version}/${lock.linux64.executable}`);
  const file = (uid, mode) => () => ({ isFile: () => true, uid, mode: 0o100000 | mode });
  assert.equal(candidates({}, file(0, 0o755))[0], pinnedChromium);
  assert.equal(candidates({}, file(1000, 0o755))[0], "/usr/bin/chromium", "a user-owned file is never preferred");
  assert.equal(candidates({}, () => { throw new Error("missing"); })[0], "/usr/bin/chromium");
});

test("MP-11 blocked schemes", () => {
  for (const url of ["chrome://settings/", "devtools://devtools/x", "file:///etc/passwd", "view-source:https://a.example/", "chrome-untrusted://x/", "chrome://extensions-internals/"]) assert(blockedUrl(url), url);
  for (const url of ["https://example.com/", "about:blank", "", null, "not a url", "chrome://extensions/", "chrome://extensions/?id=abc", "chrome-extension://abc/popup.html"]) assert(!blockedUrl(url), String(url));
  for (const url of ["chrome://extensions/", "chrome-extension://abc/options.html", "https://chromewebstore.google.com/detail/x/abc", "https://chrome.google.com/webstore/detail/abc", "CHROMEWEBSTORE.google.com/category/extensions"]) assert(extensionManagementUrl(url), url);
  for (const url of ["https://example.com/", "https://google.com/webstore", "https://chromewebstore.google.com.evil.example/", "not a url", null]) assert(!extensionManagementUrl(url), String(url));
});
