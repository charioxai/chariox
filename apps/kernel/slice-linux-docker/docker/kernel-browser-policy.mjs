// MP-11: the kernel's own Chromium is locked down by kernel-owned managed
// policies. They are visible only inside its private mount namespace, never in
// a system-wide policy directory (a person's own Chrome must not inherit them).
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { TAB_GROUPS_EXTENSION_ID } from "./kernel-browser-tab-groups.mjs";

// The native tab strip/omnibox is shown to the person (BROWSER-UX-14). Miguel
// 2026-10-15: Chrome's own settings and internal pages, DevTools, downloads and
// printing are allowed (agents may need those tools where they work; files land
// on the kernel's machine). Files and view-source, Chrome's password manager (it
// would copy Vault-filled passwords into the profile), other profiles, sign-in
// and fullscreen stay off. Extensions are the person's own, from the Web Store,
// as in any Chrome; agents cannot reach extension management (host refusal,
// extensionManagementUrl). DeveloperToolsAvailability stays default: its
// blocking value also refuses the kernel's own CDP attach to every page.
export const managedPolicy = Object.freeze({
  URLBlocklist: ["file://*", "view-source:*"],
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
const blockedSchemes = new Set(["file:", "view-source:"]);
export function blockedUrl(url) {
  if (typeof url !== "string") return false;
  try { return blockedSchemes.has(new URL(url).protocol); } catch { return false; }
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


export const EXTENSIONS_REFUSED = "MP-11: extensions are the person's own; an agent cannot open the Web Store or chrome://extensions or act on them";

// MP-11 review #1001 P1: Computer can click an unfocused window or a native install
// confirmation. A cached active Tab is insufficient. Fence the whole owned desktop
// while ANY live extension surface is open; an unreadable inventory cannot prove safety.
export async function admitComputerMutation(host, command) {
  if (!(command._agent_input || command.focused_agent) || !['input', 'target_action', 'launch_application'].includes(command.op)) return;
  const browser = host.browser;
  if (!browser && !host.chromium.child) return; // Native applications without a browser.
  try {
    if (!browser || host.chromium.child?.exitCode !== null || host.chromium.child?.signalCode !== null) throw new Error();
    const connection = await browser.ensureConnection();
    const { targetInfos } = await connection.send('Target.getTargets');
    if (host.browser !== browser || connection.isOpen?.() === false || !Array.isArray(targetInfos) || !targetInfos.length) throw new Error();
    let destinations = 0;
    for (const target of targetInfos) {
      if (['browser', 'tab', 'service_worker', 'shared_worker', 'worker'].includes(target.type)) continue;
      destinations++;
      // Empty/unparseable URLs occur while a target is being created. Do not input
      // through that transition. Worker targets include our tab-groups extension.
      new URL(target.url);
      if (extensionManagementUrl(target.url) || target.pendingUrl && extensionManagementUrl(target.pendingUrl)) throw new Error();
    }
    if (!destinations) throw new Error();
  } catch { throw new Error(EXTENSIONS_REFUSED); }
}
