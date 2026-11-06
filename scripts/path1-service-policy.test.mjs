import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { verifyPath1ServicePolicy } from "../deploy/managed-kernel/path1-service-policy.mjs"

const unit = readFileSync(new URL("../deploy/managed-kernel/chariox-path1-managed-bootstrap.service", import.meta.url), "utf8")
test("MP-01/MP-07/MP-11 home ordinary service retains its deployment policy", () => {
  assert.doesNotThrow(() => verifyPath1ServicePolicy(unit))
})
for (const directive of ["TemporaryFileSystem=/home:ro", "IPAddressDeny=any", "PrivateMounts=yes"]) {
  test(`MP-01/MP-07/MP-11 home service rejects inherited ${directive}`, () => {
    for (const section of ["Service", "Unit"]) {
      const restricted = unit.replace(`[${section}]`, `[${section}]\n${directive}`)
      assert.throws(() => verifyPath1ServicePolicy(restricted), {
        message: new RegExp(`contains ${directive.split("=")[0]}=`),
      })
    }
  })
}
