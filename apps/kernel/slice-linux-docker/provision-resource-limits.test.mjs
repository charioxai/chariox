import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const apparmorProfile = fileURLToPath(new URL("./chariox-slice-provider.apparmor", import.meta.url))

test("a configured slice memory limit does not acquire extra swap", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /--memory \"\$SLICE_DOCKER_MEMORY\"/)
  assert.match(source, /--memory-swap \"\$SLICE_DOCKER_MEMORY\"/)
})

test("nested provider namespaces can use a host-installed AppArmor profile", async () => {
  const [source, profile] = await Promise.all([
    readFile(provisioner, "utf8"),
    readFile(apparmorProfile, "utf8"),
  ])

  assert.match(source, /CHARIOX_SLICE_APPARMOR_PROFILE/)
  assert.match(source, /--security-opt apparmor="\$SLICE_APPARMOR_PROFILE"/)
  assert.match(profile, /profile chariox-slice-provider flags=\(unconfined\)/)
  assert.match(profile, /^\s*userns,\s*$/m)
})
