// MD-2: Linux host policy. The renderer sandbox is never disabled.
import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";
// install-root.sh installs the pinned Chromium (deploy/local-linux/kernel-browser-chromium.lock.json)
// read-only under /usr/lib/chariox; the kernel prefers it over a system browser.
export const pinnedChromium = "/usr/lib/chariox/browser/chromium/155.0.8059.79/chrome-linux64/chrome";
function rootOwned(file, stat) {
  try { const entry = stat(file); return entry.isFile() && entry.uid === 0 && (entry.mode & 0o022) === 0; } catch { return false; }
}
export function candidates(_environment, stat = statSync) {
  return [...(rootOwned(pinnedChromium, stat) ? [pinnedChromium] : []), "/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"];
}
export function launchEnvironment(environment, uid = process.getuid?.()) {
  if (uid === 0) throw new Error("MD-2: run the kernel browser as a normal Unix user; root Chromium is unsupported");
  return environment;
}
// MP-11: Chromium reads managed policies only from its compiled /etc path. Run it
// in a private user+mount namespace whose /etc is a read-only overlay of the
// kernel's policy layer over the real /etc, then map back to the caller's own
// uid/gid (Chromium refuses root) and keep Chromium's namespace sandbox. Each
// step execs, so the spawned PID stays the browser's PID. The kernel's private
// temporary directory becomes /tmp there: a short singleton socket path that the
// namespace can open (it cannot open the kernel's /proc/<pid>/fd).
// A new mount namespace holds locked copies of every host mount, and a locked copy
// of an App's storage mount keeps its loop device busy after the storage helper
// releases it (stop fails, the App stays Failed). So the namespace starts from a
// fresh tmpfs root that rbinds every top-level tree except the App storage root
// and pivots into it (cwd too, or it would pin the old tree); the old root, with all
// inherited copies, is detached.
const script = [
  'set -e; launcher=$1; layer=$2; temporary=$3; uid=$4; gid=$5; storage=$6; shift 6',
  'pivot=/usr/sbin/pivot_root; [ -x "$pivot" ] || pivot=/usr/bin/pivot_root; new=$temporary/.root; mkdir "$new"; /usr/bin/mount -t tmpfs tmpfs "$new"; /usr/bin/mount --make-unbindable "$new"',
  'copy() { for e in "$1"/* "$1"/.[!.]*; do',
  '  if [ -L "$e" ]; then ln -s "$(readlink "$e")" "$2${e#"$1"}"; continue; fi',
  '  [ -d "$e" ] || continue; [ "$e" = "$storage" ] && continue',
  '  mkdir "$2${e#"$1"}"',
  '  case $storage in "$e"/*) copy "$e" "$2${e#"$1"}";; *) /usr/bin/mount --rbind "$e" "$2${e#"$1"}";; esac',
  'done; }',
  'copy "" "$new"; mkdir "$new/.old"; "$pivot" "$new" "$new/.old"; cd /; /usr/bin/umount -l /.old; rmdir /.old; rmdir "$temporary/.root" || :',
  '/usr/bin/mount -t overlay overlay -o "lowerdir=$layer:/etc" /etc; /usr/bin/mount --bind "$temporary" /tmp; export TMPDIR=/tmp; exec "$launcher" --user --map-user="$uid" --map-group="$gid" -- "$@"',
].join("\n");
// Host App storage root (packages/app-runtime storage_linux.rs); never part of the browser's view.
export const appStorageRoot = "/var/lib/chariox-app-storage";
// Ubuntu >= 23.10 restricts unprivileged user namespaces through AppArmor unless a
// profile allows them (Chrome's own package installs one for its binary only).
// install-root.sh installs this root-owned copy of util-linux unshare; its AppArmor
// profile (chariox-browser-unshare) is the only userns exception the lockdown needs.
export const browserLauncher = "/usr/lib/chariox/browser/chariox-browser-unshare";
export function namespaceLauncher(stat = statSync) {
  return rootOwned(browserLauncher, stat) ? browserLauncher : "/usr/bin/unshare";
}
export function policyNamespaceAvailable(run = spawnSync, launcher = namespaceLauncher()) {
  const probe = run(launcher, ["--user", "--map-root-user", "--mount", "--", "/bin/true"], { stdio: "ignore", timeout: 5000 });
  return probe.status === 0;
}
export function policyLaunch(layer, temporary, binary, args, uid = process.getuid(), gid = process.getgid(), launcher = namespaceLauncher(), storage = appStorageRoot) {
  if (!Number.isSafeInteger(uid) || uid <= 0 || !Number.isSafeInteger(gid) || gid < 0) throw new Error("MP-11: invalid browser user");
  return { command: launcher, args: ["--user", "--map-root-user", "--mount", "--", "/bin/sh", "-c", script, "chariox-browser", launcher, layer, temporary, String(uid), String(gid), storage, binary, "--disable-setuid-sandbox", ...args] };
}
