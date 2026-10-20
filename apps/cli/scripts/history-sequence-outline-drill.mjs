#!/usr/bin/env node
// MP-08 / MP-10 / MP-11: focused regression drill for allocated local 534 / peer 142.
// The 508 sequence contracts are retained. This is supplementary to live
// terminal output records carry their history sequence, outline blobs are one
// per tool call / thought with display previews, and blob contents load in one
// batch. Live client acceptance is the PANE-01/03/06/08 drill set.
import { spawn } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..")
const kernelTests = [
  "local::api::tests::session_view::mp08_mp10_mp11_initial_view_is_a_bounded_latest_turn_not_a_blob_download",
  "local::api::tests::protocol_shapes::client_catalog_revisions::client_catalog_revisions_534_142_are_versioned_and_hashed",
  "local::api::tests::protocol_shapes::client_catalog_revisions::client_catalog_snapshot_and_change_event_534_142_are_versioned_and_hashed",
  "terminal::stream::tests::service::stalled_attachment_retains_six_thousand_history_deltas",
  "terminal::stream::tests::service::output_records_preserve_distinct_history_sequences",
  "runtime::history_requests::session_outline::tests::batch_blob_contents::initial_outline_batches_three_agents_and_marks_budgeted_steers",
  "local::api::tests::protocol_shapes::panes_history::panes_history_508_range_steering_and_initial_content_snapshot",
  "runtime::state::provider_output_runtime_tests::structured_output::structured_output_batch_persists_one_turn_id_for_all_chunks",
  "runtime::agent_actor::tests::prompt_command_execution::queued_controls::prompt_steer_queued_removes_queue_entry_without_settling_active_prompt",
  "runtime::history_requests::session_outline::projection::tests::tool_blobs_are_one_per_call_and_never_span_inline_text",
  "runtime::history_requests::session_outline::projection::tests::reasoning_blobs_are_one_per_thought_with_its_first_line",
  "runtime::history_requests::session_outline::tests::batch_blob_contents::blob_contents_batch_returns_each_requested_blob_in_order",
  "runtime::history_requests::session_outline::tests::split_tool_calls::tool_call_split_by_a_sentence_stays_one_blob",
  "local::api::tests::protocol_shapes::recall_terminal_metaagent::local_daemon_protocol_terminal_output_shape_is_versioned",
  "local::api::tests::protocol_shapes::workspace_history_external::local_daemon_protocol_session_history_outline_shape_is_versioned",
  "lib_tests::client_protocol_conformance::local_daemon_protocol_version_matches_typescript_kernel_client",
]
const steps = [
  ...kernelTests.map((name) => ({
    name,
    command: "cargo",
    args: ["test", "--manifest-path", "apps/kernel/Cargo.toml", "--lib", name, "--", "--exact"],
    // An exact filter that matches nothing exits 0; the drill must have run the test.
    mustRun: 1,
  })),
  { name: "kernel client build and tests", command: "pnpm", args: ["--filter", "@chariox/kernel-client", "run", "test"] },
]

for (const step of steps) {
  console.log(`[history-sequence-outline-drill] ${step.name}`)
  await run(step.command, step.args, step.mustRun)
}
console.log("[history-sequence-outline-drill] ok")

function run(command, args, mustRun = 0) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, stdio: ["inherit", mustRun ? "pipe" : "inherit", "inherit"], env: process.env })
    let output = ""
    child.stdout?.on("data", (chunk) => { output += chunk; process.stdout.write(chunk) })
    child.on("error", reject)
    child.on("exit", (code, signal) => {
      if (code !== 0) return reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}`))
      if (mustRun && !new RegExp(`test result: ok\\. ${mustRun} passed;`).test(output)) {
        return reject(new Error(`${args.at(-3)} ran ${mustRun} expected test(s) zero times; the selector is stale`))
      }
      resolve()
    })
  })
}
