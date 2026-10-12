#!/usr/bin/env node
// MP-11 F7: exercise the real Rust serialization boundary without provider credentials.
import { spawnSync } from "node:child_process"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"

export function runPublicProviderRunProtocolDrill(binary, run = spawnSync) {
  if (!binary) throw new Error("MP-11 F7 requires --test-binary pointing to a prebuilt kernel lib-test executable")
  const checks = [
    "provider::public_run::tests::mp11_f7_public_provider_run_protocol_435_snapshot",
    "provider::public_run::tests::mp11_f7_public_run_responses_and_events_never_emit_launch_secrets",
    "provider::public_run::tests::mp11_f7_native_endpoint_rejects_embedded_credentials",
  ]
  const results = checks.map(check => {
    const result = run(resolve(binary), [check, "--exact"], {
      encoding: "utf8", timeout: 60_000,
      env: { ...process.env, RUST_MIN_STACK: "16777216", RUST_TEST_THREADS: "1" },
      maxBuffer: 1024 * 1024,
    })
    // Retain only fixed public check names and counts, never test diagnostic payloads.
    const passed = result.status === 0 && /test result: ok\. 1 passed; 0 failed;/.test(result.stdout ?? "")
    return { check, passed, exitCode: result.status }
  })
  return { mpItem: "MP-11", finding: "F7", localProtocol: 487, scope: "synthetic public DTO/response/event serialization and private persistence", results, passed: results.every(result => result.passed) }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const index = process.argv.indexOf("--test-binary")
  try {
    const report = runPublicProviderRunProtocolDrill(index >= 0 ? process.argv[index + 1] : undefined)
    console.log(JSON.stringify(report, null, 2))
    process.exitCode = report.passed ? 0 : 1
  } catch {
    console.error("MP-11 F7 protocol drill failed; supply a prebuilt kernel lib-test executable with --test-binary")
    process.exitCode = 1
  }
}
