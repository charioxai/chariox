// MP-07/MP-08/MP-10: regressions must run through the CI pnpm test path.
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

test("MP-07/MP-08 review regression files are selected by CI", async () => {
  const root = JSON.parse(await readFile(new URL("../package.json", import.meta.url)))
  const cli = JSON.parse(await readFile(new URL("../apps/cli/package.json", import.meta.url)))
  assert.match(root.scripts.test, /pnpm run test:review-regressions/)
  const command = root.scripts["test:review-regressions"]
  for (const file of [
    "deploy/managed-kernel/managed-update-recovery.test.mjs",
    "apps/kernel/managed-upgrade-protocol-transitions.test.mjs",
    "apps/cli/scripts/lib/drill-turn-admission.test.mjs",
    "apps/cli/scripts/lib/native-permission-workspace.test.mjs",
  ]) assert.ok(command?.includes(file), `${file} must execute in CI`)
  assert.match(command, /--test-concurrency=1/)
  assert.ok(cli.scripts.test, "workspace CLI tests remain selected")
})

test("MP-08 setup-token run documents protocol 411 and the shared login", async () => {
  const docs = await readFile(new URL("../docs/PROVIDER_ACCOUNTS.md", import.meta.url), "utf8")
  const paragraph = docs.split("\n").find(line => line.includes("Create setup token"))
  assert.match(paragraph, /protocol 411 or newer/)
  assert.match(paragraph, /setup_token/)
})

test("MP-07 CI executes the recovery ownership fixture as root", async () => {
  const ci = await readFile(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8")
  assert.match(ci, /sudo "\$\(command -v node\)" --test --test-concurrency=1 deploy\/managed-kernel\/managed-update-recovery\.test\.mjs/)
})
