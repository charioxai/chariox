import { mkdtempSync, chmodSync, lstatSync, readFileSync, unlinkSync, rmdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn, spawnSync } from "node:child_process"
import { readLocalDevEnrollment, verifyInstalledLocalSource } from "./protected-local-docker-authority.mjs"
import { watchLocalBrokerLifetime } from "./local-docker-broker-lifetime.mjs"
import { LocalBrokerRefusal, awaitLocalBrokerTransport } from "./local-docker-broker-transport.mjs"

// Only the ordinary home kernel starts this launcher. No private input is placed
// in argv, environment, image or transport directory.
const uid = process.getuid()
const parent = process.ppid
const parentBirth = () => {
  try { return readFileSync(`/proc/${parent}/stat`, "utf8").split(") ").at(-1).split(" ")[19] } catch { return undefined }
}
const originalParentBirth = parentBirth()
let directory, child, container
let cleaned = false
const env = {PATH: "/usr/bin:/bin", HOME: "/nonexistent", DOCKER_HOST: "unix:///run/docker.sock"}
function cleanup() {
  if (cleaned) return
  cleaned = true
  if (container) spawnSync("/usr/bin/docker", ["stop", "-t", "3", container], {env, stdio: "ignore", timeout: 10_000})
  // This exact mkdtemp directory contains only the owned public transport socket.
  if (directory) {
    const socket = join(directory, "control.sock")
    try { if (lstatSync(socket).isSocket()) unlinkSync(socket) } catch (error) { if (error.code !== "ENOENT") throw error }
    try { rmdirSync(directory) } catch (error) { if (!["ENOENT", "ENOTEMPTY"].includes(error.code)) throw error }
  }
}
const quit = watchLocalBrokerLifetime(cleanup,
  () => Boolean(originalParentBirth) && parentBirth() === originalParentBirth)
try {
  const enrollment = readLocalDevEnrollment(uid)
  verifyInstalledLocalSource(enrollment)
  const socketIdentity = lstatSync("/run/docker.sock")
  if (!socketIdentity.isSocket() || socketIdentity.uid !== enrollment.socket.uid
      || socketIdentity.gid !== enrollment.socket.gid || (socketIdentity.mode & 0o777) !== enrollment.socket.mode) throw new LocalBrokerRefusal("Docker socket differs from the enrollment")
  directory = mkdtempSync(join(tmpdir(), `chariox-local-broker-${uid}-`))
  chmodSync(directory, 0o700)
  const socket = join(directory, "control.sock")
  container = `chariox-local-broker-${uid}-${directory.split("-").at(-1).toLowerCase()}`
  // Node is not an init: without --init, orphaned grandchildren (for example the
  // provisioner's timeout watchdog sleeps) become unreaped zombies that keep
  // counting against --pids-limit until the helper can no longer start tasks.
  child = spawn("/usr/bin/docker", ["run", "--rm", "--init", "--name", container,
    "--read-only", "--network", "none", "--user", "0:0", "--cap-drop", "ALL",
    "--cap-add", "CHOWN", "--cap-add", "DAC_OVERRIDE", "--cap-add", "FOWNER",
    "--security-opt", "no-new-privileges", "--memory", "512m", "--cpus", "2", "--pids-limit", "512",
    "--tmpfs", "/tmp:rw,nosuid,nodev,size=128m", "--tmpfs", "/run/chariox-slice-broker:rw,nosuid,nodev,size=32m",
    "--mount", "type=bind,src=/run/docker.sock,dst=/run/docker.sock",
    "--mount", `type=bind,src=${enrollment.sourceRoot},dst=${enrollment.sourceRoot},readonly`,
    "--mount", `type=bind,src=/etc/chariox/slice-local-dev/${uid}.json,dst=/etc/chariox/slice-local-dev/${uid}.json,readonly`,
    "--mount", `type=bind,src=${enrollment.controlRoot.slice(0, -7)},dst=${enrollment.controlRoot.slice(0, -7)}`,
    "--mount", `type=bind,src=${directory},dst=${directory}`,
    "-e", `CHARIOX_SLICE_LOCAL_DEV_OWNER_UID=${uid}`, "-e", `CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME=${container}`,
    "-e", `CHARIOX_SLICE_LOCAL_DEV_SOCKET_IDENTITY=${JSON.stringify({dev: socketIdentity.dev, ino: socketIdentity.ino})}`,
    "-e", "DOCKER_HOST=unix:///run/docker.sock", "-e", `CHARIOX_SLICE_DOCKER_BROKER_SOCKET=${socket}`,
    "-e", `CHARIOX_SLICE_DOCKER_SHARE_ROOT=${enrollment.controlRoot}/share`,
    "-e", `CHARIOX_SLICE_DOCKER_BROKER_OUTPUT_ROOT=${enrollment.controlRoot}/share/.broker-private/output`,
    "-e", `CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT=${enrollment.controlRoot}/share/.broker-private/artifacts`,
    "-e", `CHARIOX_SLICE_DOCKER_HANDLE_ROOT=${enrollment.controlRoot}/handles`,
    "-e", `CHARIOX_SLICE_DOCKER_HANDLE_STATE=${enrollment.controlRoot}/handles.json`,
    "--entrypoint", "/usr/local/bin/node", enrollment.helperImageId,
    `${enrollment.sourceRoot}/apps/kernel/slice-linux-docker/managed-docker-broker.mjs`], {env, stdio: "ignore"})
  child.once("error", quit)
  child.once("exit", quit)
  await awaitLocalBrokerTransport(socket, {ownerUid: uid, helperExited: () => child.exitCode !== null})
  process.stdout.write(`${socket}\n`)
} catch (error) {
  cleanup()
  // The kernel logs this line; only refusals carry a reason.
  const reason = error instanceof LocalBrokerRefusal ? `: ${error.message}` : ""
  process.stderr.write(`Verified local Docker DEV broker startup refused${reason}\n`)
  process.exit(1)
}
