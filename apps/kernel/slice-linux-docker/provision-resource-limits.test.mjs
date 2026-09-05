import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const apparmorProfile = fileURLToPath(new URL("./chariox-slice-provider.apparmor", import.meta.url))
const dockerfile = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
const bubblewrapLauncher = fileURLToPath(new URL("./docker/managed-provider-bwrap.sh", import.meta.url))
const seccompSource = fileURLToPath(new URL("./docker/managed-provider-seccomp.c", import.meta.url))
const runtime = fileURLToPath(new URL("./docker/start-runtime.sh", import.meta.url))

test("a configured slice memory limit does not acquire extra swap", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /--memory \"\$SLICE_DOCKER_MEMORY\"/)
  assert.match(source, /--memory-swap \"\$SLICE_DOCKER_MEMORY\"/)
})

test("the shared slice disk reserve reaches the browser controller container", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /-e "CHARIOX_SLICE_MIN_FREE_MB=\$SLICE_MIN_FREE_MB"/)
})

test("provider listener ranges are reserved from container ephemeral ports", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(
    source,
    /--sysctl \"net\.ipv4\.ip_local_reserved_ports=\$SLICE_CODEX_PORT_RANGE,\$SLICE_OPENCODE_PORT_RANGE\"/,
  )
})

test("nested provider namespaces can use a host-installed AppArmor profile", async () => {
  const [source, profile, image, launcher, seccomp, runtimeSource] = await Promise.all([
    readFile(provisioner, "utf8"),
    readFile(apparmorProfile, "utf8"),
    readFile(dockerfile, "utf8"),
    readFile(bubblewrapLauncher, "utf8"),
    readFile(seccompSource, "utf8"),
    readFile(runtime, "utf8"),
  ])

  assert.match(source, /CHARIOX_SLICE_APPARMOR_PROFILE/)
  assert.match(source, /--cap-add SYS_ADMIN/)
  assert.match(source, /--cap-add NET_ADMIN/)
  assert.match(source, /--cap-add SYS_PTRACE/)
  assert.match(source, /--security-opt apparmor="\$SLICE_APPARMOR_PROFILE"/)
  assert.match(image, /chmod 4755 \/usr\/bin\/bwrap/)
  assert.match(launcher, /\/usr\/bin\/bwrap --seccomp 3/)
  assert.match(seccomp, /SCMP_SYS\(unshare\)/)
  assert.match(seccomp, /SCMP_SYS\(clone3\)/)
  assert.match(runtimeSource, /CHARIOX_MANAGED_PROVIDER_BWRAP="\/usr\/local\/libexec\/chariox\/managed-provider-bwrap"/)
  assert.match(profile, /profile chariox-slice-provider flags=\(unconfined\)/)
  assert.match(profile, /^\s*userns,\s*$/m)
})
