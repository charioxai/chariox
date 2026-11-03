import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { verifyPath1ServicePolicy } from "./path1-service-policy.mjs"

// MP-01/MP-07/MP-11: two review anchors describe this one parser defect.
const unit = await readFile(new URL("chariox-path1-managed-bootstrap.service", import.meta.url), "utf8")
test(`home accepts canonical and whitespace-normalized assignments`, () => {
  verifyPath1ServicePolicy(unit)
  verifyPath1ServicePolicy(unit.replace(/^([A-Za-z][A-Za-z0-9]*)=(.*)$/gm, "$1 \t= \t$2"))
})
for (const assignment of [
  // MP-01/MP-07/MP-11: restrictions outside the old sandbox deny list.
  "NoExecPaths=/home /tmp",
  "ExecPaths=/usr/bin",
  "ProtectProc=invisible",
  "ProcSubset=pid",
  "RestrictFileSystems=ext4",
  "MemoryDenyWriteExecute=yes",
  "SystemCallArchitectures=native",
  "RestrictNetworkInterfaces=lo",
  "DevicePolicy=closed",
  "FutureProviderSandbox=yes",
  "ProtectHome = yes",
  "TemporaryFileSystem = /home:ro",
  "IPAddressDeny = any",
  "User = root",
  "Group \t= root",
  "Environment = HOME=/tmp/wrong-home",
  "EnvironmentFile = /tmp/synthetic-override",
  "PassEnvironment = HOME",
  "UnsetEnvironment = HOME",
  "ExecStart = /tmp/untrusted",
  "StateDirectory = unexpected",
]) {
  test(`home rejects final spaced override ${assignment}`, () => {
    assert.throws(() => verifyPath1ServicePolicy(`${unit}\n[Service]\n${assignment}\n`))
  })
}
for (const syntax of [
  "[Service]\nEnvironment=HOME=/tmp/\\\nwrong-home",
  "[Service]\nUser = chariox\\\nroot",
  "[Service]\nmalformed assignment",
  "User=root\n[Service]",
]) {
  test(`home rejects unsupported syntax ${JSON.stringify(syntax)}`, () => {
    assert.throws(() => verifyPath1ServicePolicy(`${syntax}\n${unit}`))
  })
}
