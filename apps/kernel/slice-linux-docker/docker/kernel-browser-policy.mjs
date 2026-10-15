// MP-11: the kernel's own Chromium is locked down by kernel-owned managed
// policies. They are visible only inside its private mount namespace, never in
// a system-wide policy directory (a person's own Chrome must not inherit them).
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { TAB_GROUPS_EXTENSION_ID } from "./kernel-browser-tab-groups.mjs";

// The native tab strip/omnibox is shown to the person (BROWSER-UX-14): its
// internal pages, developer tools, files, downloads, password manager, other
// profiles and printing must not reach the kernel's machine. Extensions are the
// person's own, from the Web Store, as in any Chrome (Miguel 2026-10-15); agents
// cannot reach extension management (host refusal, extensionManagementUrl).
// DeveloperToolsAvailability 2 is not usable: it also refuses the kernel's own
// CDP attach to every page. DevTools stays closed through the devtools://
// blocklist (Chrome then refuses DevTools on every page) and the kernel's
// fail-closed target layer.
export const managedPolicy = Object.freeze({
  // Chrome draws the omnibox dropdown and tab search as chrome://*.top-chrome pages;
  // PDFs open in its built-in viewer component extension; the kernel's own
  // tab-groups extension runs as one; chrome://extensions manages the person's extensions.
  URLAllowlist: ["chrome://omnibox-popup.top-chrome", "chrome://tab-search.top-chrome", "chrome://extensions", "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai", `chrome-extension://${TAB_GROUPS_EXTENSION_ID}`],
  URLBlocklist: ["chrome://*", "chrome-untrusted://*", "devtools://*", "file://*", "view-source:*"],
  DownloadRestrictions: 3,
  PasswordManagerEnabled: false,
  AutofillAddressEnabled: false,
  AutofillCreditCardEnabled: false,
  BrowserSignin: 0,
  SyncDisabled: true,
  // Extensions come from the Web Store only (none external or sideloaded) besides the kernel's
  // own, loaded over its CDP pipe. ExtensionInstallBlocklist "*" is not used: Chrome then
  // refuses that load even for an allow-listed id.
  ExtensionInstallSources: ["https://chromewebstore.google.com/*"],
  BlockExternalExtensions: true,
  // Its icon is never pinned in the native toolbar (Chrome for Testing pins it by default).
  ExtensionSettings: { [TAB_GROUPS_EXTENSION_ID]: { toolbar_pin: "default_unpinned" } },
  IncognitoModeAvailability: 1,
  // MP-11: a same-size fullscreen window would put page pixels in the band crop.
  FullscreenAllowed: false,
  BrowserGuestModeEnabled: false,
  BrowserAddPersonEnabled: false,
  PrintingEnabled: false,
  // A web app install would add launchers on the kernel's machine.
  WebAppInstallByUserEnabled: false,
  TaskManagerEndProcessEnabled: false,
  DefaultBrowserSettingEnabled: false,
  CommandLineFlagSecurityWarningsEnabled: false,
  // Ctrl+T and the home button open the same home page as the web client's New tab.
  NewTabPageLocation: "https://www.google.com/",
  HomepageLocation: "https://www.google.com/",
  DefaultSearchProviderEnabled: true,
  DefaultSearchProviderName: "DuckDuckGo",
  DefaultSearchProviderKeyword: "duckduckgo.com",
  DefaultSearchProviderSearchURL: "https://duckduckgo.com/?q={searchTerms}",
  DefaultSearchProviderSuggestURL: "https://duckduckgo.com/ac/?q={searchTerms}&type=list",
});

// Compiled-in policy roots under /etc: Chromium, Google Chrome, Chrome for Testing.
const policyDirectories = ["chromium/policies/managed", "opt/chrome/policies/managed", "opt/chrome_for_testing/policies/managed"];

// Writes the overlay layer that becomes /etc inside the browser's namespace.
// An https home page without credentials may replace the default (drills: DuckDuckGo).
function homePolicy(environment) {
  try {
    const url = new URL(environment.CHARIOX_KERNEL_BROWSER_HOME_URL ?? "");
    if (url.protocol === "https:" && !url.username && !url.password) return { NewTabPageLocation: url.href, HomepageLocation: url.href };
  } catch {}
  return {};
}
export async function writePolicyLayer(root, environment = process.env) {
  const layer = path.join(root, "policy-etc");
  // overlayfs options are comma/colon separated: such a path cannot be a layer.
  if (!path.isAbsolute(layer) || /[,:\\\s]/.test(layer)) throw new Error("MP-11: browser policy layer path is not mountable");
  await rm(layer, { recursive: true, force: true });
  for (const directory of policyDirectories) {
    await mkdir(path.join(layer, directory), { recursive: true, mode: 0o755 });
    await writeFile(path.join(layer, directory, "chariox.json"), JSON.stringify({ ...managedPolicy, ...homePolicy(environment) }), { mode: 0o644 });
  }
  return layer;
}

// Second, fail-closed layer in the kernel: a page that still reaches a blocked
// scheme (policy missing or bypassed) is closed.
const blockedSchemes = new Set(["chrome:", "chrome-untrusted:", "devtools:", "file:", "view-source:"]);
export function blockedUrl(url) {
  if (typeof url !== "string") return false;
  try { const u = new URL(url); return blockedSchemes.has(u.protocol) && !(u.protocol === "chrome:" && u.hostname === "extensions"); } catch { return false; }
}

// MP-11: the person's extension management (Web Store, chrome://extensions, extension pages).
// Agents are refused it this iteration; installs by agents come later through /sudo.
export function extensionManagementUrl(url) {
  if (typeof url !== "string") return false;
  let u;
  try { u = new URL(/^[a-z][a-z0-9+.-]*:/i.test(url) ? url : `https://${url}`); } catch { return false; }
  const web = u.protocol === "https:" || u.protocol === "http:";
  return u.protocol === "chrome-extension:" || u.protocol === "chrome:" && u.hostname === "extensions"
    || web && (u.hostname === "chromewebstore.google.com" || u.hostname === "chrome.google.com" && /^\/webstore(\/|$)/.test(u.pathname));
}
