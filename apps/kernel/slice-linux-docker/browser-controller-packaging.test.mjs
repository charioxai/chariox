import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const dockerRoot = path.resolve(fileURLToPath(new URL("./docker/", import.meta.url)))
const controllerEntry = path.join(dockerRoot, "browser-controller.mjs")
const dockerfilePath = path.join(dockerRoot, "Dockerfile")
const provisionerPath = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

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
