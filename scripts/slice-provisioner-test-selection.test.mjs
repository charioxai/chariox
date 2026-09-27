import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const sliceTestDirectory = "apps/kernel/slice-linux-docker"
const selectionMarkerVariable = "CHARIOX_SLICE_PROVISIONER_SELECTION_MARKER"

function selectionCommand() {
  const packageJson = JSON.parse(readFileSync(join(repositoryRoot, "package.json"), "utf8"))
  return packageJson.scripts["test:slice-provisioner"]
}

function selectionGlobs(command) {
  const tokens = command.trim().split(/\s+/)
  assert.deepEqual(tokens.slice(0, 3), ["node", "--test", "--test-concurrency=1"])
  assert.doesNotMatch(command, /--test-(?:skip|name)-pattern/)
  return tokens.slice(3)
}

function expandOneLevelGlob(pattern) {
  const separator = pattern.lastIndexOf("/")
  const directory = resolve(repositoryRoot, pattern.slice(0, separator))
  const filenamePattern = pattern.slice(separator + 1)
  const wildcard = filenamePattern.indexOf("*")
  assert.notEqual(wildcard, -1, `expected a shell glob: ${pattern}`)
  assert.equal(filenamePattern.indexOf("*", wildcard + 1), -1, `expected one wildcard: ${pattern}`)
  const prefix = filenamePattern.slice(0, wildcard)
  const suffix = filenamePattern.slice(wildcard + 1)
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.startsWith(prefix) && entry.name.endsWith(suffix))
    .map((entry) => join(pattern.slice(0, separator), entry.name))
    .sort()
}

function addFixtureTest(directory, filename, { fail = false } = {}) {
  const source = [
    'import { appendFileSync } from "node:fs"',
    'import test from "node:test"',
    `appendFileSync(process.env.${selectionMarkerVariable}, ${JSON.stringify(`${filename}\n`)})`,
    `test(${JSON.stringify(filename)}, () => { ${fail ? 'throw new Error("intentional selection failure")' : ""} })`,
  ].join("\n")
  writeFileSync(join(directory, filename), source)
}

function makeSelectionFixture(t, quotaTests) {
  const root = mkdtempSync(join(tmpdir(), "slice-provisioner-selection-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const testDirectory = join(root, sliceTestDirectory)
  mkdirSync(testDirectory, { recursive: true })
  writeFileSync(join(root, "package.json"), JSON.stringify({
    private: true,
    scripts: { "test:slice-provisioner": selectionCommand() },
  }))
  addFixtureTest(testDirectory, "provision-existing.test.mjs")
  for (const quotaTest of quotaTests) addFixtureTest(testDirectory, quotaTest.filename, quotaTest)
  return { root, marker: join(root, "selection-marker") }
}

function runSelectionFixture(root, marker) {
  const env = { ...process.env }
  delete env.NODE_TEST_CONTEXT
  env[selectionMarkerVariable] = marker
  env.CI = "1"
  env.NO_COLOR = "1"
  const result = spawnSync("pnpm", ["run", "test:slice-provisioner"], {
    cwd: root,
    encoding: "utf8",
    env,
    maxBuffer: 1024 * 1024,
    timeout: 20_000,
  })
  const diagnostics = [
    `status: ${result.status}`,
    `signal: ${result.signal}`,
    `error: ${result.error ?? "none"}`,
    `stdout:\n${result.stdout ?? ""}`,
    `stderr:\n${result.stderr ?? ""}`,
  ].join("\n")
  assert.equal(result.error, undefined, `pnpm fixture must start successfully\n${diagnostics}`)
  assert.equal(result.signal, null, `pnpm fixture should exit normally\n${diagnostics}`)
  return { ...result, diagnostics }
}

test("slice provisioner selection covers its dynamic repository inventory once and sequentially", () => {
  const patterns = selectionGlobs(selectionCommand())
  const provisionPattern = `${sliceTestDirectory}/provision-*.test.mjs`
  const quotaPattern = `${sliceTestDirectory}/slice-disk-quota*.test.mjs`
  assert.deepEqual(patterns, [provisionPattern, quotaPattern])

  const selected = patterns.flatMap(expandOneLevelGlob)
  const directory = resolve(repositoryRoot, sliceTestDirectory)
  const testFiles = readdirSync(directory).filter((filename) => filename.endsWith(".test.mjs"))
  const provisionInventory = testFiles.filter((filename) => filename.startsWith("provision-"))
  const quotaInventory = testFiles.filter((filename) => filename.includes("quota"))

  assert.ok(provisionInventory.length > 0, "the provisioner test inventory must not be empty")
  assert.ok(quotaInventory.length > 0, "the quota test inventory must not be empty")
  assert.equal(new Set(selected).size, selected.length, "a test file must not be selected twice")
  for (const filename of [...provisionInventory, ...quotaInventory]) {
    assert.ok(selected.includes(join(sliceTestDirectory, filename)), `${filename} must be selected`)
  }
  assert.deepEqual(
    selected,
    [...new Set([...provisionInventory, ...quotaInventory])]
      .map((filename) => join(sliceTestDirectory, filename))
      .sort(),
    "the command should select all provisioner and quota tests and no unrelated files",
  )
})

test("selection runs future quota-pattern files once and propagates a selected test failure", (t) => {
  const fixture = makeSelectionFixture(t, [
    { filename: "slice-disk-quota-current.test.mjs" },
    { filename: "slice-disk-quota-future.test.mjs", fail: true },
  ])
  const result = runSelectionFixture(fixture.root, fixture.marker)
  const output = `${result.stdout}\n${result.stderr}`

  assert.notEqual(result.status, 0, `a failing selected quota test must fail the package script\n${result.diagnostics}`)
  assert.match(output, /intentional selection failure/, result.diagnostics)
  assert.match(output, /(?:#|ℹ)\s*tests 3\b/, result.diagnostics)
  assert.match(output, /(?:#|ℹ)\s*pass 2\b/, result.diagnostics)
  assert.match(output, /(?:#|ℹ)\s*fail 1\b/, result.diagnostics)
  assert.match(output, /(?:#|ℹ)\s*skipped 0\b/, result.diagnostics)
  assert.deepEqual(
    readFileSync(fixture.marker, "utf8").trim().split("\n").sort(),
    [
      "provision-existing.test.mjs",
      "slice-disk-quota-current.test.mjs",
      "slice-disk-quota-future.test.mjs",
    ].sort(),
    `each matching provisioner and quota test file must execute exactly once\n${result.diagnostics}`,
  )
})

test("an empty quota glob cannot turn the package script into a zero-quota-test success", (t) => {
  const fixture = makeSelectionFixture(t, [])
  const result = runSelectionFixture(fixture.root, fixture.marker)

  // The runner can reject the unmatched quota path before it executes the provisioner fixture.
  assert.notEqual(result.status, 0, `an unmatched quota glob must not be silently ignored\n${result.diagnostics}`)
})
