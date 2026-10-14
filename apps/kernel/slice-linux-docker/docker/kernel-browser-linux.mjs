// MD-2: Linux host policy. The renderer sandbox is never disabled.
import { spawnSync } from "node:child_process";
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
const script = 'set -e; layer=$1; uid=$2; gid=$3; shift 3; /usr/bin/mount -t overlay overlay -o "lowerdir=$layer:/etc" /etc; exec /usr/bin/unshare --user --map-user="$uid" --map-group="$gid" -- "$@"';
// Ubuntu >= 23.10 restricts unprivileged user namespaces through AppArmor unless a
// profile allows them (Chrome's own package installs one for its binary only).
export function policyNamespaceAvailable(run = spawnSync) {
  const probe = run("/usr/bin/unshare", ["--user", "--map-root-user", "--mount", "--", "/bin/true"], { stdio: "ignore", timeout: 5000 });
  return probe.status === 0;
}
export function policyLaunch(layer, binary, args, uid = process.getuid(), gid = process.getgid()) {
  if (!Number.isSafeInteger(uid) || uid <= 0 || !Number.isSafeInteger(gid) || gid < 0) throw new Error("MP-11: invalid browser user");
  return { command: "/usr/bin/unshare", args: ["--user", "--map-root-user", "--mount", "--", "/bin/sh", "-c", script, "chariox-browser", layer, String(uid), String(gid), binary, "--disable-setuid-sandbox", ...args] };
}
