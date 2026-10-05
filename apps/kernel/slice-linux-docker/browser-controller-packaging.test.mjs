import assert from "node:assert/strict"
import { copyFile, mkdtemp, readFile, rm } from "node:fs/promises"
import { execFile } from "node:child_process"
import { tmpdir } from "node:os"
import { promisify } from "node:util"
import path from "node:path"
import test from "node:test"
import { fileURLToPath, pathToFileURL } from "node:url"

const dockerRoot = path.resolve(fileURLToPath(new URL("./docker/", import.meta.url)))
const controllerEntry = path.join(dockerRoot, "browser-controller.mjs")
const dockerfilePath = path.join(dockerRoot, "Dockerfile")
const provisionerPath = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

test("MP-08/MP-10 refreshed controller imports load from an empty legacy support directory", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const refresh = source.slice(source.indexOf("refresh_slice_support_files() {"), source.indexOf("\nwait_for_container_running()"))
  async function loadOverlays(text) {
    const root = await mkdtemp(path.join(tmpdir(), "chariox-browser-overlay-"))
    try {
      // Stage exactly the local controller files named by the real refresh.
      for (const match of text.matchAll(/\$REPO_ROOT\/apps\/kernel\/slice-linux-docker\/docker\/([^"\s]+\.mjs)/g)) {
        await copyFile(path.join(dockerRoot, match[1]), path.join(root, match[1]))
      }
      for (const name of ["actions", "cdp", "snapshot"]) {
        await promisify(execFile)(process.execPath, ["--input-type=module", "--eval",
          "await import(process.argv[1])", pathToFileURL(path.join(root, `browser-controller-${name}.mjs`)).href])
      }
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  }
  for (const name of ["actionability", "interactions", "geometry"]) {
    const missing = refresh.split("\n").filter(line => !line.includes(`docker/browser-controller-${name}.mjs`)).join("\n")
    await assert.rejects(loadOverlays(missing), error => /ERR_MODULE_NOT_FOUND/.test(error.stderr),
      `MP-10 missing ${name} must reproduce an unresolved import on upgrade`)
  }
  await loadOverlays(refresh)
})

test("slice packaging installs every browser controller runtime module", async () => {
  const [modules, dockerfile, provisioner] = await Promise.all([
    reachableControllerModules(controllerEntry),
    readFile(dockerfilePath, "utf8"),
    readFile(provisionerPath, "utf8"),
  ])

  assert.ok(modules.size > 1, "fixture should discover browser controller dependencies")
  for (const modulePath of modules) {
    const name = path.basename(modulePath)
    assert.match(
      dockerfile,
      new RegExp(`docker/${escapeRegExp(name)}\\s+/opt/chariox-slice/${escapeRegExp(name)}`),
      `Dockerfile should install ${name}`,
    )
    assert.match(
      provisioner,
      new RegExp(`docker/${escapeRegExp(name)}["']?\\s+[^\\n]*/opt/chariox-slice/${escapeRegExp(name)}`),
      `live slice refresh should install ${name}`,
    )
  }
})

test("slice packaging installs the Computer text finder", async () => {
  const [dockerfile, provisioner] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(provisionerPath, "utf8"),
  ])

  assert.match(
    dockerfile,
    /docker\/slice-text-finder\.py\s+\/opt\/chariox-slice\/slice-text-finder\.py/,
  )
  assert.match(
    provisioner,
    /docker\/slice-text-finder\.py["']?\s+[^\n]*\/opt\/chariox-slice\/slice-text-finder\.py/,
  )
})

test("slice packaging installs the private browser import destination runtime", async () => {
  const [dockerfile, provisioner,controller] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(provisionerPath, "utf8"),
    readFile(controllerEntry,"utf8"),
  ])
  for (const name of ["chrome-cookie-batch.mjs", "controller-cookie-import.mjs",
    "cookie-import-completion.mjs", "cookie-import-journal.mjs",
    "cookie-import-transaction.mjs", "production-destination.mjs"]) {
    assert.match(dockerfile,new RegExp(`apps/browser-session-import/${escapeRegExp(name)}\\s+`))
    assert.match(provisioner,new RegExp(escapeRegExp(name)))
  }
  assert.match(controller,/request\.method === "browser\.cookies\.recover"/)
  assert.match(controller,/recoverProductionBrowserImport/)
})

async function reachableControllerModules(entry) {
  const pending = [entry]
  const modules = new Set()
  while (pending.length > 0) {
    const modulePath = pending.pop()
    if (modules.has(modulePath)) continue
    modules.add(modulePath)
    const source = await readFile(modulePath, "utf8")
    for (const match of source.matchAll(/from\s+["'](\.\/[^"]+?\.mjs)["']/g)) {
      const dependency = path.resolve(path.dirname(modulePath), match[1])
      assert.equal(path.dirname(dependency), dockerRoot, `controller import escaped ${dockerRoot}`)
      pending.push(dependency)
    }
  }
  return modules
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}


test("physical display and capture dependencies are required recovery overlays", async () => {
  const [dockerfile, provisioner] = await Promise.all([
    readFile(dockerfilePath, "utf8"), readFile(provisionerPath, "utf8"),
  ])
  for (const name of ["canonical-display.py", "canonical_vnc.py", "xorg-dummy.conf", "selkies-capture.py"]) {
    assert.match(dockerfile, new RegExp(`docker/${escapeRegExp(name)}\\s+/opt/chariox-slice/${escapeRegExp(name)}`))
    assert.match(provisioner, new RegExp(`copy_required_slice_overlay[^\\n]*docker/${escapeRegExp(name)}[^\\n]*/opt/chariox-slice/${escapeRegExp(name)}`))
  }
})

// MP-08 / MP-11: fail closed on refreshed images missing the target guard.
test("slice packaging installs the physical keyboard and Computer secret target guard", async () => {
  const [dockerfile, provisioner] = await Promise.all([
    readFile(dockerfilePath, "utf8"), readFile(provisionerPath, "utf8"),
  ])
  assert.match(dockerfile, /docker\/slice-keyboard\.py\s+\/opt\/chariox-slice\/slice-keyboard\.py/)
  assert.match(provisioner, /copy_required_slice_overlay[^\n]*docker\/slice-keyboard\.py[^\n]*\/opt\/chariox-slice\/slice-keyboard\.py/)
})
