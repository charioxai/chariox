// MP-11: the kernel's own Chromium is locked down by kernel-owned managed
// policies. They are visible only inside its private mount namespace, never in
// a system-wide policy directory (a person's own Chrome must not inherit them).
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";

// The native tab strip/omnibox is shown to the person (BROWSER-UX-14): its
// internal pages, developer tools, extensions, files, downloads, password
// manager, other profiles and printing must not reach the kernel's machine.
// DeveloperToolsAvailability 2 is not usable: it also refuses the kernel's own
// CDP attach to every page. DevTools stays closed through the devtools://
// blocklist (Chrome then refuses DevTools on every page) and the kernel's
// fail-closed target layer.
export const managedPolicy = Object.freeze({
  // Chrome draws the omnibox dropdown and tab search as chrome://*.top-chrome pages;
  // PDFs open in its built-in viewer component extension.
  URLAllowlist: ["chrome://omnibox-popup.top-chrome", "chrome://tab-search.top-chrome", "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai"],
  URLBlocklist: ["chrome://*", "chrome-untrusted://*", "devtools://*", "chrome-extension://*", "file://*", "view-source:*"],
  DownloadRestrictions: 3,
  PasswordManagerEnabled: false,
  AutofillAddressEnabled: false,
  AutofillCreditCardEnabled: false,
  BrowserSignin: 0,
  SyncDisabled: true,
  ExtensionInstallBlocklist: ["*"],
  IncognitoModeAvailability: 1,
  BrowserGuestModeEnabled: false,
  BrowserAddPersonEnabled: false,
  PrintingEnabled: false,
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
export async function writePolicyLayer(root) {
  const layer = path.join(root, "policy-etc");
  // overlayfs options are comma/colon separated: such a path cannot be a layer.
  if (!path.isAbsolute(layer) || /[,:\\\s]/.test(layer)) throw new Error("MP-11: browser policy layer path is not mountable");
  await rm(layer, { recursive: true, force: true });
  for (const directory of policyDirectories) {
    await mkdir(path.join(layer, directory), { recursive: true, mode: 0o755 });
    await writeFile(path.join(layer, directory, "chariox.json"), JSON.stringify(managedPolicy), { mode: 0o644 });
  }
  return layer;
}

// Second, fail-closed layer in the kernel: a page that still reaches a blocked
// scheme (policy missing or bypassed) is closed.
const blockedSchemes = new Set(["chrome:", "chrome-untrusted:", "devtools:", "chrome-extension:", "file:", "view-source:"]);
export function blockedUrl(url) {
  if (typeof url !== "string") return false;
  try { return blockedSchemes.has(new URL(url).protocol); } catch { return false; }
}
