import assert from "node:assert/strict"
import test from "node:test"
import {
  assertProjectQuotaTreeMatches,
  checkProjectQuotaTree,
  parseProjectQuotaState,
  readProjectQuotaIds,
  readProjectQuotaRow,
} from "./slice-disk-quota-xfs-readback.mjs"

// These are synthetic upstream-derived fixtures from xfsprogs v6.16.0 source,
// not live target-image captures. The Ubuntu 26.04 image's installed xfsprogs
// version is unpinned; the root-owned Linux campaign must capture that output.
const STATE_ON = `Project quota state on /var/lib/chariox-docker/data (/dev/loop0)
 Accounting: ON
 Enforcement: ON
 Inode: N/A
Blocks grace time: [--------]
Blocks max warnings: 5`

const GOOD_PROJECT_CHECK = `Checking project 1073741824 (path /var/lib/chariox-docker/data/volumes/home/_data)...
Processed 1 (/dev/null and cmdline) paths for project 1073741824 with recursion depth infinite (-1).`

const PROJECT_ROW = "#1073741824 128 0 1024 00 [--------]"

function fixedOutput(expectedExpression, output, calls) {
  return (mountpoint, expression) => {
    calls.push({ mountpoint, expression })
    assert.equal(expression, expectedExpression)
    return output
  }
}

test("production state parser accepts the exact project accounting and enforcement block", () => {
  assert.deepEqual(parseProjectQuotaState(STATE_ON), { accounting: true, enforcement: true })
  assert.deepEqual(parseProjectQuotaState(STATE_ON.replace("Accounting: ON", "Accounting: OFF")), {
    accounting: false,
    enforcement: true,
  })
  assert.throws(() => parseProjectQuotaState(STATE_ON.replace("Enforcement: ON\n", "")), /missing or duplicated/)
  assert.throws(() => parseProjectQuotaState(`${STATE_ON}\n${STATE_ON.split("\n")[0]}\n Accounting: ON\n Enforcement: ON`), /missing or duplicated/)
  assert.throws(() => parseProjectQuotaState(STATE_ON.replace(" Accounting: ON\n", " Accounting: ON\n Accounting: OFF\n")), /missing or duplicated/)
  assert.throws(() => parseProjectQuotaState(STATE_ON.replace("Accounting: ON", "Accounting: MAYBE")), /accounting state is malformed/)
  assert.deepEqual(parseProjectQuotaState(`${STATE_ON}\nUser quota state on /var/lib/chariox-docker/data (/dev/loop0)\n Accounting: OFF\n Enforcement: OFF`), {
    accounting: true,
    enforcement: true,
  })
})

test("production project-tree check accepts normal Checking and Processed output", () => {
  const calls = []
  assert.equal(assertProjectQuotaTreeMatches(
    fixedOutput("project -c -p /var/lib/chariox-docker/data/volumes/home/_data 1073741824", GOOD_PROJECT_CHECK, calls),
    "/var/lib/chariox-docker/data",
    "/var/lib/chariox-docker/data/volumes/home/_data",
    1_073_741_824,
  ), true)
  assert.deepEqual(calls, [{
    mountpoint: "/var/lib/chariox-docker/data",
    expression: "project -c -p /var/lib/chariox-docker/data/volumes/home/_data 1073741824",
  }])
})

test("production project-tree check reports known mismatch as false and rejects malformed output", () => {
  const mismatched = `Checking project 1073741824 (path /var/lib/chariox-docker/data/volumes/home/_data)...
/var/lib/chariox-docker/data/volumes/home/_data/file - project identifier is not set (inode=42, tree=1073741824)
Processed 1 (/dev/null and cmdline) paths for project 1073741824 with recursion depth infinite (-1).`
  const inheritedMismatch = `Checking project 1073741824 (path /var/lib/chariox-docker/data/volumes/home/_data)...
/var/lib/chariox-docker/data/volumes/home/_data - project inheritance flag is not set
Processed 1 (/dev/null and cmdline) paths for project 1073741824 with recursion depth infinite (-1).`
  assert.equal(checkProjectQuotaTree(() => mismatched, "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), false)
  assert.equal(checkProjectQuotaTree(() => inheritedMismatch, "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), false)
  assert.throws(() => assertProjectQuotaTreeMatches(() => mismatched, "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), /mismatched project assignment/)
  assert.throws(() => assertProjectQuotaTreeMatches(() => GOOD_PROJECT_CHECK.split("\n")[0], "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), /malformed/)
  assert.throws(() => assertProjectQuotaTreeMatches(() => `${GOOD_PROJECT_CHECK}\nwarning`, "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), /malformed/)
  const duplicateMismatch = `${mismatched.split("\n").slice(0, -1).join("\n")}\n/var/lib/chariox-docker/data/volumes/home/_data/file - project identifier is not set (inode=42, tree=1073741824)\n${mismatched.split("\n").at(-1)}`
  assert.throws(() => checkProjectQuotaTree(() => duplicateMismatch, "/data", "/var/lib/chariox-docker/data/volumes/home/_data", 1_073_741_824), /duplicate mismatch/)
})

test("production quota-row readback selects a bounded project report and converts KiB exactly", () => {
  const calls = []
  const row = readProjectQuotaRow(
    fixedOutput("report -p -b -n -N -L 1073741824 -U 1073741824", PROJECT_ROW, calls),
    "/var/lib/chariox-docker/data",
    1_073_741_824,
    { required: true },
  )
  assert.deepEqual(row, {
    projectId: 1_073_741_824,
    found: true,
    usedBytes: 131_072,
    hardLimitBytes: 1_048_576,
  })
  assert.deepEqual(calls, [{
    mountpoint: "/var/lib/chariox-docker/data",
    expression: "report -p -b -n -N -L 1073741824 -U 1073741824",
  }])
})

test("production quota-row readback rejects required missing, malformed, duplicate, mismatched, and overflowing rows", () => {
  const read = (output, options = { required: true }) =>
    readProjectQuotaRow(() => output, "/data", 1_073_741_824, options)

  assert.throws(() => read(""), /missing/)
  assert.deepEqual(read("", { required: false }), {
    projectId: 1_073_741_824,
    found: false,
    usedBytes: 0,
    hardLimitBytes: 0,
  })
  assert.throws(() => read("1073741824 128 0 1024 00 [--------]"), /malformed/)
  assert.throws(() => read("#1073741824 0128 0 1024 00 [--------]"), /malformed/)
  assert.throws(() => read("#1073741824 128 0 1024 0 [--------]"), /malformed/)
  assert.throws(() => read(`${PROJECT_ROW}\n${PROJECT_ROW}`), /duplicate rows/)
  assert.throws(() => read(PROJECT_ROW.replace("#1073741824", "#1073741825")), /different project ID/)

  const tooLargeKiB = (BigInt(Number.MAX_SAFE_INTEGER) / 1024n + 1n).toString()
  assert.throws(() => read(`#1073741824 ${tooLargeKiB} 0 1024 00 [--------]`), /safe integer range/)
  assert.throws(() => read(`#1073741824 128 0 ${tooLargeKiB} 00 [--------]`), /safe integer range/)
})

test("production ID census parses #numeric IDs and rejects duplicate or malformed rows", () => {
  const calls = []
  const ids = readProjectQuotaIds(
    fixedOutput("report -p -b -n -N", `${PROJECT_ROW}\n#1073741825 0 0 2048 00 [--none--]`, calls),
    "/var/lib/chariox-docker/data",
  )
  assert.deepEqual(ids, [1_073_741_824, 1_073_741_825])
  assert.deepEqual(calls, [{ mountpoint: "/var/lib/chariox-docker/data", expression: "report -p -b -n -N" }])
  assert.throws(() => readProjectQuotaIds(() => `${PROJECT_ROW}\n${PROJECT_ROW}`, "/data"), /duplicate project IDs/)
  assert.throws(() => readProjectQuotaIds(() => "device 128 0 1024 00 [--------]", "/data"), /malformed/)
})
