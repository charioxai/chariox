import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import fs from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const probe = fileURLToPath(new URL("./managed-provider-isolation-probe.mjs", import.meta.url))

// Exercise the executable's failure/finally path without a live kernel or account.
const socketFixture = `
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
module.exports = class extends EventEmitter {
  constructor() { super(); queueMicrotask(() => this.emit('open')); }
  send(raw) {
    const { request_id, request } = JSON.parse(raw);
    let response = {};
    if (request.CreateSession) response = { SessionCreated: { session: { id: 'fixture' } } };
    if (request.LaunchProviderRun) {
      const workspace = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE;
      if (process.env.PROBE_CASE !== 'missing') fs.writeFileSync(
        process.env.CHARIOX_MANAGED_ISOLATION_PROBE_RESULT,
        process.env.PROBE_CASE === 'success'
          ? 'managed_provider_isolation=ok\\nreal_provider=/codex\\nworkspace=' + workspace + '\\noutside_repository=/tmp/repo\\noutside_clone=/tmp/clone\\n'
          : 'managed_provider_isolation=failure\\nreason=probe account missing\\n',
        { mode: 0o600 });
      response = { ProviderRunLaunched: { provider_run: { id: 'run' } } };
    }
    if (request.GetProviderRun) response = { ProviderRun: { provider_run: {
      id: 'run', state: process.env.PROBE_CASE === 'success' ? 'Running' : 'Ended'
    } } };
    if (request.EndSession) fs.writeFileSync(process.env.CLEANUP_MARKER, 'ended');
    queueMicrotask(() => this.emit('message', JSON.stringify({ type: 'response', request_id, response })));
  }
  close() { this.emit('close'); }
};
`

for (const scenario of ["failure", "success", "missing"]) {
  test(`probe ${scenario}: preserve failure evidence and still end the session`, async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "chariox-probe-evidence-"))
    try {
      await fs.mkdir(path.join(root, "node_modules/ws"), { recursive: true })
      await fs.writeFile(path.join(root, "package.json"), "{}")
      await fs.writeFile(path.join(root, "node_modules/ws/index.js"), socketFixture)
      const resultPath = path.join(root, "result")
      const cleanupMarker = path.join(root, "ended")
      const child = spawnSync(process.execPath, [probe], {
        encoding: "utf8", timeout: 5000,
        env: {
          PATH: process.env.PATH,
          PROBE_CASE: scenario,
          CLEANUP_MARKER: cleanupMarker,
          CHARIOX_PROBE_PACKAGE_JSON: path.join(root, "package.json"),
          CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "fixture-only",
          CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE: root,
          CHARIOX_MANAGED_ISOLATION_PROBE_RESULT: resultPath,
          CHARIOX_PROBE_TIMEOUT_MS: "1000",
        },
      })
      assert.ifError(child.error)
      assert.equal(await fs.readFile(cleanupMarker, "utf8"), "ended")
      assert.equal(child.status, scenario === "success" ? 0 : 1, child.stderr)
      if (scenario === "failure") {
        assert.match(await fs.readFile(resultPath, "utf8"), /reason=probe account missing/)
        assert.match(child.stderr, /wrapper_result=.*result/)
      } else {
        await assert.rejects(fs.stat(resultPath), { code: "ENOENT" })
      }
      if (scenario === "missing") assert.match(child.stderr, /provider run entered Ended/)
    } finally {
      await fs.rm(root, { recursive: true, force: true })
    }
  })
}
