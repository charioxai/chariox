// MD-2: Linux host policy. The renderer sandbox is never disabled.
import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";
export function candidates() {
  return ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"];
}
export function launchEnvironment(environment, uid = process.getuid?.()) {
  if (uid === 0) throw new Error("MD-2: run the kernel browser as a normal Unix user; root Chromium is unsupported");
  return environment;
}
// MP-11: Chromium reads managed policies only from its compiled /etc path. Run it
// in a private user+mount namespace whose /etc is a read-only overlay of the
// kernel's policy layer over the real /etc, then map back to the caller's own
// uid/gid (Chromium refuses root) and keep Chromium's namespace sandbox. Each
// step execs, so the spawned PID stays the browser's PID.
const script = 'set -e; launcher=$1; layer=$2; uid=$3; gid=$4; shift 4; /usr/bin/mount -t overlay overlay -o "lowerdir=$layer:/etc" /etc; exec "$launcher" --user --map-user="$uid" --map-group="$gid" -- "$@"';
// Ubuntu >= 23.10 restricts unprivileged user namespaces through AppArmor unless a
// profile allows them (Chrome's own package installs one for its binary only).
// install-root.sh installs this root-owned copy of util-linux unshare; its AppArmor
// profile (chariox-browser-unshare) is the only userns exception the lockdown needs.
export const browserLauncher = "/usr/lib/chariox/browser/chariox-browser-unshare";
export function namespaceLauncher(stat = statSync) {
  try {
    const file = stat(browserLauncher);
    if (file.isFile() && file.uid === 0 && (file.mode & 0o022) === 0) return browserLauncher;
  } catch {}
  return "/usr/bin/unshare";
}
export function policyNamespaceAvailable(run = spawnSync, launcher = namespaceLauncher()) {
  const probe = run(launcher, ["--user", "--map-root-user", "--mount", "--", "/bin/true"], { stdio: "ignore", timeout: 5000 });
  return probe.status === 0;
}
export function policyLaunch(layer, binary, args, uid = process.getuid(), gid = process.getgid(), launcher = namespaceLauncher()) {
  if (!Number.isSafeInteger(uid) || uid <= 0 || !Number.isSafeInteger(gid) || gid < 0) throw new Error("MP-11: invalid browser user");
  return { command: launcher, args: ["--user", "--map-root-user", "--mount", "--", "/bin/sh", "-c", script, "chariox-browser", launcher, layer, String(uid), String(gid), binary, "--disable-setuid-sandbox", ...args] };
}
