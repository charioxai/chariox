use std::collections::BTreeMap;

use crate::provider::LaunchProviderRequest;

pub(super) fn dev_stub_pty_args(model: &str) -> Vec<String> {
    let script = match model {
        "workflow-drill-node-1" => Some(dev_stub_workflow_output_script(
            "workflow drill node 1",
            1842,
        )),
        "workflow-drill-node-2" => Some(dev_stub_workflow_output_script(
            "workflow drill node 2",
            1843,
        )),
        "workflow-single-turn-node" => Some(dev_stub_workflow_read_through_script(
            "workflow single turn node",
            1842,
        )),
        "workflow-intermediate-node" => Some(dev_stub_workflow_intermediate_script(
            "workflow intermediate node",
            1841,
            1842,
        )),
        "workflow-dashboard-producer-node" => Some(dev_stub_workflow_dashboard_producer_script()),
        "workflow-final-passthrough-node" => Some(dev_stub_workflow_final_passthrough_script()),
        "workflow-html-final-node" => Some(dev_stub_workflow_html_final_script()),
        "workflow-code-topology-node" => Some(dev_stub_workflow_code_topology_script()),
        "semantic-url-renderer-stub" => Some(dev_stub_semantic_renderer_script()),
        "slow-first-output-drill" => Some(dev_stub_slow_first_output_script()),
        "large-output-drill" => Some(dev_stub_large_output_script()),
        "metaagent-workflow-code-author" => Some(dev_stub_metaagent_workflow_code_author_script()),
        "terminal-echo-a" | "terminal-echo-b" => Some(dev_stub_terminal_echo_script()),
        "model-source" | "workflow-test-idle" => Some(dev_stub_native_tui_idle_script()),
        "native-tui-idle" => Some(dev_stub_native_tui_idle_script()),
        _ => None,
    }
    .unwrap_or_else(|| "cat".to_string());
    vec!["-lc".to_string(), script]
}

fn is_dev_stub_workflow_drill_model(model: &str) -> bool {
    matches!(
        model,
        "workflow-drill-node-1"
            | "workflow-drill-node-2"
            | "workflow-single-turn-node"
            | "workflow-intermediate-node"
            | "workflow-dashboard-producer-node"
            | "workflow-final-passthrough-node"
            | "workflow-html-final-node"
            | "workflow-code-topology-node"
            | "semantic-url-renderer-stub"
            | "slow-first-output-drill"
            | "large-output-drill"
            | "metaagent-workflow-code-author"
    )
}

pub(super) fn is_dev_stub_unique_pty_model(model: &str) -> bool {
    is_dev_stub_workflow_drill_model(model)
        || matches!(model, "terminal-echo-a" | "terminal-echo-b")
}

fn dev_stub_terminal_echo_script() -> String {
    "while IFS= read -r line; do printf '%s\\n' \"$line\"; done".to_string()
}

fn dev_stub_native_tui_idle_script() -> String {
    "stty -echo 2>/dev/null || true; (sleep 3600; kill $$ 2>/dev/null) & while :; do IFS= read -r _line || sleep 1; done"
        .to_string()
}

fn dev_stub_workflow_html_final_script() -> String {
    let html = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><style>body{margin:0;font-family:Inter,system-ui,sans-serif;background:#101418;color:#f7fbff}.dashboard{min-height:100vh;padding:28px;display:grid;gap:18px;grid-template-columns:repeat(3,minmax(0,1fr))}.tile{border:1px solid rgba(255,255,255,.2);border-radius:12px;padding:18px;background:linear-gradient(135deg,#ff4d6d,#2ec4b6)}.wide{grid-column:span 2;background:linear-gradient(135deg,#7b2ff7,#f107a3)}h1{grid-column:1/-1;margin:0;font-size:32px}.metric{font-size:42px;font-weight:800}@media(max-width:760px){.dashboard{grid-template-columns:1fr}.wide{grid-column:auto}}</style></head><body><main class="dashboard"><h1>Vibrant Workflow Dashboard</h1><section class="tile wide"><p>Revenue pulse</p><div class="metric">$84.2K</div></section><section class="tile"><p>Conversion</p><div class="metric">18%</div></section><section class="tile"><p>Latency</p><div class="metric">42ms</div></section><section class="tile"><p>Users</p><div class="metric">9,481</div></section></main></body></html>"#;
    let payload = serde_json::json!({
        "summary": "generated vibrant dashboard html",
        "output": {
            "message": {
                "kind": "html",
                "html": html
            }
        }
    })
    .to_string();
    format!(
        "stty -echo 2>/dev/null || true; seen=' '; while IFS= read -r _line; do token=$(printf '%s' \"$_line\" | grep -Eo 'workflow-ack:[A-Za-z0-9_-]+' | tail -n 1); key=${{token:-__no_token__}}; case \"$seen\" in *\" $key \"*) ;; *) sleep 2; printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; seen=\"$seen$key \";; esac; done"
    )
}

pub(super) fn dev_stub_pty_env(request: &LaunchProviderRequest) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if matches!(
        request.model.as_str(),
        "workflow-intermediate-node"
            | "workflow-dashboard-producer-node"
            | "workflow-final-passthrough-node"
            | "workflow-code-topology-node"
            | "metaagent-workflow-code-author"
    ) {
        if let Some(binding) = request.runtime_mcp_binding.as_ref() {
            env.insert(
                "ARROBA_DEV_STUB_RUNTIME_MCP_URL".to_string(),
                binding.server_url.clone(),
            );
            env.insert(
                "ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN".to_string(),
                binding.auth_token.clone(),
            );
        }
    }
    env
}

fn dev_stub_workflow_code_topology_script() -> String {
    let js = r#"const http = require("node:http");
const runtimeUrl = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_URL;
const runtimeToken = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN;
const emittedDeliveryTokens = new Set();
let promptBuffer = "";
let pendingTimer = null;

function callRuntimeTool(name, args = {}) {
  return new Promise((resolve, reject) => {
    if (!runtimeUrl || !runtimeToken) return reject(new Error("runtime MCP binding missing"));
    const parsed = new URL(runtimeUrl);
    const body = JSON.stringify({
      jsonrpc: "2.0",
      id: `workflow-code-topology-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method: "tools/call",
      params: { name, arguments: args },
    });
    const req = http.request({
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname || "/mcp",
      method: "POST",
      headers: {
        authorization: `Bearer ${runtimeToken}`,
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      },
    }, (res) => {
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        if (res.statusCode !== 200) return reject(new Error(`runtime MCP HTTP ${res.statusCode}: ${text}`));
        const response = JSON.parse(text);
        if (response.error) return reject(new Error(JSON.stringify(response.error)));
        if (response.result && response.result.isError) return reject(new Error(JSON.stringify(response.result)));
        resolve(response.result && (response.result.structuredContent || response.result.payload) || response.result);
      });
    });
    req.setTimeout(5000, () => {
      req.destroy(new Error(`runtime MCP ${name} timeout`));
    });
    req.on("error", reject);
    req.end(body);
  });
}

function parseDeliveryToken() {
  const matches = [...promptBuffer.matchAll(/workflow-ack:[A-Za-z0-9_-]+/g)];
  return matches.length ? matches[matches.length - 1][0] : undefined;
}

function text(value) {
  return String(value || "").toLowerCase();
}

function inboundCount(context) {
  return Array.isArray(context && context.messages) ? context.messages.length : 0;
}

function currentTurnIndex() {
  const matches = [...promptBuffer.matchAll(/This is turn (\d+) for this node/g)];
  if (!matches.length) return 1;
  return Number(matches[matches.length - 1][1]) || 1;
}

function selectFinalOutput() {
  const prompt = text(promptBuffer);
  if (prompt.includes("refine the draft")) {
    return { answer: "refined draft accepted", changes: ["tightened structure", "preserved requirements"] };
  }
  if (prompt.includes("answer code")) {
    return { answer: "code specialist handled the routed implementation task", specialist: "code" };
  }
  if (prompt.includes("answer analysis") || prompt.includes("research specialist")) {
    return { answer: "research specialist handled the routed analysis task", specialist: "research" };
  }
  if (prompt.includes("synthesize them")) {
    return { answer: "synthesized findings from both workers", source_count: 2 };
  }
  if (prompt.includes("aggregate their votes")) {
    return { decision: "pass", rationale: "both reviewers returned passing structured outputs", reviewer_count: 2 };
  }
  if (prompt.includes("survives critique")) {
    return { decision: "accept", rationale: "proposal survived one critique loop" };
  }
  if (prompt.includes("filtered candidates into the final result")) {
    return { result: "selected candidate alpha", selected_count: 1 };
  }
  if (prompt.includes("accepted, submit final output")) {
    return { answer: "accepted after one revision", iterations: 2 };
  }
  if (prompt.includes("accepted final output") || prompt.includes("evaluate the candidate")) {
    return { answer: "optimized answer accepted", accepted: true };
  }
  if (prompt.includes("compare them") || prompt.includes("tournament")) {
    return { winner: "a", reason: "contestant a gave the clearer structured answer" };
  }
  if (prompt.includes("combine the orchestration context")) {
    return { answer: "delegated worker result consolidated", delegated: true };
  }
  return { answer: "workflow-code topology completed" };
}

function payloadForEdge(edge, index, context) {
  const target = text(edge.target_instructions);
  const messages = inboundCount(context);
  if (target.includes("refine the draft")) {
    return { draft: "first complete draft", notes: ["ready for refinement"] };
  }
  if (target.includes("answer code")) {
    return { task: "implement the requested code path", reason: "prompt asks for repository implementation" };
  }
  if (target.includes("answer analysis") || target.includes("research")) {
    return { task: "analyze the requested topic", reason: "prompt asks for research" };
  }
  if (target.includes("work your assigned angle")) {
    return { question: "validate the workflow-code fan-out behavior", angle: index === 0 ? "evidence" : "risk" };
  }
  if (target.includes("wait for all worker inputs")) {
    return { finding: "worker finding for synthesis", evidence: [`worker-${index + 1}`] };
  }
  if (target.includes("review the task from the policy") || target.includes("review the task from the quality")) {
    return { subject: "parallel workflow-code review", criteria: ["correctness", "coverage"] };
  }
  if (target.includes("wait for both reviewer results")) {
    return { verdict: "pass", notes: [`reviewer-${index + 1} passed`] };
  }
  if (target.includes("find flaws")) {
    return { claim: "workflow-code proposal is testable", evidence: ["deterministic stub", "schema validation"] };
  }
  if (target.includes("produce a proposal")) {
    return { issues: ["tighten evidence"], recommendation: "revise" };
  }
  if (target.includes("decide whether the proposal")) {
    return { issues: ["resolved after revision"], recommendation: "judge" };
  }
  if (target.includes("filter the candidate list")) {
    return { items: [{ value: "alpha", score: 0.92 }, { value: "beta", score: 0.37 }] };
  }
  if (target.includes("turn the filtered candidates")) {
    return { selected: ["alpha"], rationale: "highest score and complete rationale" };
  }
  if (target.includes("produce a tournament entry")) {
    return { task: "solve the toy tournament prompt", slot: index === 0 ? "a" : "b" };
  }
  if (target.includes("wait for both entries")) {
    return { answer: `contestant ${index === 0 ? "a" : "b"} answer`, strategy: "direct structured response" };
  }
  if (target.includes("complete the assigned subtask")) {
    return { subtask: "inspect workflow-code portability", acceptance_criteria: ["structured result", "no open blocker"] };
  }
  if (target.includes("combine the orchestration context")) {
    return { result: "worker completed delegated subtask", open_questions: [] };
  }
  if (target.includes("evaluate the candidate")) {
    return { candidate: messages > 0 ? "revised candidate" : "initial candidate", version: messages > 0 ? 2 : 1 };
  }
  if (target.includes("produce an improved candidate")) {
    return { decision: "revise", feedback: "improve specificity before acceptance" };
  }
  if (target.includes("if work is insufficient")) {
    return { artifact: messages > 0 ? "revised artifact" : "initial artifact", iteration: messages > 0 ? 2 : 1 };
  }
  if (target.includes("create or revise the artifact")) {
    return { status: "revise", notes: "one revision required before completion" };
  }
  return { value: index + 1, note: "generic workflow-code topology handoff" };
}

function shouldSendEdge(edge, index, context) {
  const target = text(edge.target_instructions);
  const turn = currentTurnIndex();
  if (promptBuffer.includes("Classify the request") || promptBuffer.includes("Classify")) {
    return target.includes("answer code") || (!target.includes("research") && index === 0);
  }
  if (target.includes("produce a proposal")) {
    return turn <= 1;
  }
  if (target.includes("decide whether the proposal")) {
    return turn > 1;
  }
  if (target.includes("produce an improved candidate")) {
    return turn <= 1;
  }
  if (target.includes("create or revise the artifact")) {
    return turn <= 1;
  }
  return true;
}

async function emitForDeliveryToken(deliveryToken) {
  if (deliveryToken) {
    await callRuntimeTool("ack_workflow_turn", { delivery_token: deliveryToken });
  }
  const context = await callRuntimeTool("read_workflow_turn_context", {
    ...(deliveryToken ? { delivery_token: deliveryToken } : {}),
  });
  const edges = Array.isArray(context && context.outgoing_edges) ? context.outgoing_edges : [];
  const selectedEdges = edges.filter((edge, index) => shouldSendEdge(edge, index, context));
  if (selectedEdges.length > 0) {
    const workflow_handoffs = selectedEdges.map((edge, index) => ({
      edge_id: edge.edge_id,
      message: payloadForEdge(edge, index, context),
    }));
    const payload = {
      summary: `workflow-code topology handoff to ${workflow_handoffs.length} edge(s)`,
      output: { message: { workflow_handoffs } },
    };
    process.stdout.write("```json\n" + JSON.stringify(payload) + "\n```\n");
    return;
  }
  const output = selectFinalOutput();
  await callRuntimeTool("validate_and_submit_workflow_run_output", {
    workflow_output_json: JSON.stringify(output),
    ...(deliveryToken ? { delivery_token: deliveryToken } : {}),
  });
  const payload = {
    summary: "workflow-code topology final output",
    output: { message: output },
  };
  process.stdout.write("```json\n" + JSON.stringify(payload) + "\n```\n");
}

function scheduleEmit() {
  if (pendingTimer) return;
  pendingTimer = setTimeout(() => {
    pendingTimer = null;
    const deliveryToken = parseDeliveryToken();
    if (!deliveryToken || emittedDeliveryTokens.has(deliveryToken)) return;
    emittedDeliveryTokens.add(deliveryToken);
    emitForDeliveryToken(deliveryToken).catch((error) => {
      emittedDeliveryTokens.delete(deliveryToken);
      process.stdout.write("workflow-code topology stub error: " + (error && error.message || error) + "\n");
    });
  }, 100);
}

process.stdin.setEncoding("utf8");
if (process.stdin.isTTY && typeof process.stdin.setRawMode === "function") {
  process.stdin.setRawMode(true);
}
process.stdin.on("data", (chunk) => {
  promptBuffer += chunk;
  scheduleEmit();
});
setInterval(scheduleEmit, 500);
process.stdin.resume();
"#;
    format!(
        "stty -echo 2>/dev/null || true; node -e {}",
        shell_single_quote(js)
    )
}

fn dev_stub_metaagent_workflow_code_author_script() -> String {
    let source = r#"
workflow.define({
  alias: "observed_meta_workflow_code",
  prompt: "Complete the observed metaagent workflow-code drill.",
  maxConcurrent: 2,
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "observed-meta-worker", provider: "codex", model: "gpt-5" }),
  publicLabel: "Observed worker",
  instructions: "Submit final output matching final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 0, y: 80 },
});

workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 80 } });
"#;
    let source_json = serde_json::to_string(source).expect("workflow-code source should encode");
    let js = format!(
        r#"const http = require("node:http");
const runtimeUrl = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_URL;
const runtimeToken = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN;
const source = {source_json};
let started = false;
let promptBuffer = "";

function callRuntimeTool(name, args = {{}}) {{
  return new Promise((resolve, reject) => {{
    if (!runtimeUrl || !runtimeToken) return reject(new Error("runtime MCP binding missing"));
    const parsed = new URL(runtimeUrl);
    const body = JSON.stringify({{
      jsonrpc: "2.0",
      id: `dev-stub-meta-${{Date.now()}}-${{Math.random().toString(16).slice(2)}}`,
      method: "tools/call",
      params: {{ name, arguments: args }},
    }});
    const req = http.request({{
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname || "/mcp",
      method: "POST",
      headers: {{
        authorization: `Bearer ${{runtimeToken}}`,
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      }},
    }}, (res) => {{
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {{
        const text = Buffer.concat(chunks).toString("utf8");
        if (res.statusCode !== 200) return reject(new Error(`runtime MCP HTTP ${{res.statusCode}}: ${{text}}`));
        const response = JSON.parse(text);
        if (response.error) return reject(new Error(JSON.stringify(response.error)));
        if (response.result && response.result.isError) return reject(new Error(JSON.stringify(response.result)));
        resolve(response.result ? response.result.structuredContent : response);
      }});
    }});
    req.setTimeout(5000, () => {{
      req.destroy(new Error(`runtime MCP ${{name}} timeout`));
    }});
    req.on("error", reject);
    req.end(body);
  }});
}}

function unwrap(value, key) {{
  return value && value[key] ? value[key] : value;
}}

async function authorWorkflowCode() {{
  const name = `observed-meta-workflow-code-${{Date.now()}}`;
  const providerRebindings = [{{ node: "worker", provider: "dev-stub", model: "default" }}];
  await callRuntimeTool("arroba.meta.update_task", {{
    markdown: "Observed workflow-code authoring drill: discover guidance, create, validate, apply, and run a workflow-code artifact.",
  }});
  await callRuntimeTool("arroba.meta.search_guides", {{
    query: "workflow-code authoring create validate apply run",
    tag: "workflow-code",
    command: "arroba.meta.workflow_code.create",
    limit: 5,
  }});
  await callRuntimeTool("arroba.meta.read_guide", {{ guide: "workflows/workflow-code-authoring" }});
  const created = await callRuntimeTool("arroba.meta.workflow_code.create", {{ name, source }});
  const createdArtifact = unwrap(created, "WorkflowCodeArtifactCreated").artifact;
  const validated = await callRuntimeTool("arroba.meta.workflow_code.validate", {{ name }});
  const validationOk = unwrap(validated, "WorkflowCodeValidated").result.validation.ok;
  if (!validationOk) throw new Error("workflow-code validation failed");
  const applied = await callRuntimeTool("arroba.meta.workflow_code.apply", {{
    name,
    provider_rebindings: providerRebindings,
  }});
  const appliedPayload = unwrap(applied, "WorkflowCodeApplied");
  const run = await callRuntimeTool("arroba.meta.workflow_code.run", {{
    name,
    endpoint: "entry",
    provider_rebindings: providerRebindings,
  }});
  const runPayload = unwrap(run, "WorkflowCodeRun");
  const marker = [
    "OBSERVED_WORKFLOW_CODE_AUTHORING_DONE",
    `artifact=${{name}}`,
    `workflow=${{appliedPayload.result.apply.workflow_id}}`,
    `invocation=${{runPayload.result.invocation.kind}}`,
  ].join(" ");
  await callRuntimeTool("arroba.meta.update_plan", {{
    markdown: marker,
  }});
  process.stdout.write(`${{marker}}\n`);
  process.stdout.write("```json\n" + JSON.stringify({{
    summary: "Observed metaagent workflow-code authoring completed",
    output: {{
      message: {{
        artifact: name,
        workflow_id: appliedPayload.result.apply.workflow_id,
        invocation: runPayload.result.invocation.kind,
        validation_ok: Boolean(createdArtifact && createdArtifact.metadata && createdArtifact.metadata.validation && createdArtifact.metadata.validation.ok),
      }},
    }},
  }}) + "\n```\n");
}}

function maybeStart() {{
  if (started || !promptBuffer.includes("OBSERVED_WORKFLOW_CODE_AUTHORING")) return;
  started = true;
  authorWorkflowCode().catch(async (error) => {{
    const message = `OBSERVED_WORKFLOW_CODE_AUTHORING_FAILED ${{error.message || error}}`;
    process.stdout.write(message + "\n");
    try {{
      await callRuntimeTool("arroba.meta.update_plan", {{ markdown: message }});
    }} catch {{}}
  }});
}}

process.stdin.setEncoding("utf8");
if (process.stdin.isTTY && typeof process.stdin.setRawMode === "function") {{
  process.stdin.setRawMode(true);
}}
process.stdin.on("data", (chunk) => {{
  promptBuffer += chunk;
  maybeStart();
}});
process.stdin.resume();
"#
    );
    format!(
        "stty -echo 2>/dev/null || true; node -e {}",
        shell_single_quote(&js)
    )
}

fn dev_stub_workflow_output_script(summary: &str, value: i64) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{value}}}}}}}"#);
    format!("sleep 1; printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; sleep 300")
}

fn dev_stub_workflow_read_through_script(summary: &str, value: i64) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{value}}}}}}}"#);
    format!(
        "stty -echo 2>/dev/null || true; seen=' '; while IFS= read -r _line; do token=$(printf '%s' \"$_line\" | grep -Eo 'workflow-ack:[A-Za-z0-9_-]+' | tail -n 1); key=${{token:-__no_token__}}; case \"$seen\" in *\" $key \"*) ;; *) printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; seen=\"$seen$key \";; esac; done"
    )
}

fn dev_stub_workflow_intermediate_script(
    summary: &str,
    intermediate: i64,
    final_value: i64,
) -> String {
    let final_payload = serde_json::json!({
        "summary": summary,
        "output": {
            "message": {
                "value": final_value,
            },
        },
    });
    let intermediate_output = serde_json::json!({ "value": intermediate });
    dev_stub_workflow_intermediate_payload_script(final_payload, Some(intermediate_output))
}

fn dev_stub_workflow_dashboard_producer_script() -> String {
    let final_node_instructions = [
        "You are the final workflow node.",
        "Generate the HTML yourself using the upstream dashboard_task message.",
        "Write the generated HTML artifact to dashboard.html.",
        "Emit a final fenced workflow JSON block with output.message shaped as {\"kind\":\"html_file\",\"path\":\"dashboard.html\"}.",
    ]
    .join("\n");
    let final_payload = serde_json::json!({
        "summary": "dashboard task handoff",
        "output": {
            "message": {
                "kind": "dashboard_task",
                "prompt": "Generate a vibrant dashboard as a compact self-contained HTML document.",
                "requirements": [
                    "Visible title text `Real Provider Workflow Dashboard`.",
                    "Main dashboard element has data-arroba-real-provider-dashboard=\"true\".",
                    "Inline CSS only.",
                    "No scripts, external assets, network calls, or unrelated workspace inspection.",
                    "The final workflow output must be shaped exactly as {\"kind\":\"html\",\"html\":\"<full html document>\"}.",
                ],
                "final_node_instructions": final_node_instructions,
            },
        },
    });
    dev_stub_workflow_intermediate_payload_script(final_payload, None)
}

fn dev_stub_workflow_intermediate_payload_script(
    final_payload: serde_json::Value,
    intermediate_output: Option<serde_json::Value>,
) -> String {
    let payload = final_payload.to_string();
    let intermediate_output = intermediate_output
        .map(|output| output.to_string())
        .unwrap_or_else(|| "null".to_string());
    let js = format!(
        r#"const http = require("node:http");
const runtimeUrl = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_URL;
const runtimeToken = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN;
const finalPayload = {payload_json};
const intermediateOutput = {intermediate_json};
const emittedDeliveryTokens = new Set();
let emittedWithoutToken = false;
let promptBuffer = "";
let fallbackTimer = null;

function callRuntimeTool(name, args) {{
  return new Promise((resolve, reject) => {{
    if (!runtimeUrl || !runtimeToken) return reject(new Error("runtime MCP binding missing"));
    const parsed = new URL(runtimeUrl);
    const body = JSON.stringify({{
      jsonrpc: "2.0",
      id: `dev-stub-${{Date.now()}}`,
      method: "tools/call",
      params: {{ name, arguments: args }},
    }});
    const req = http.request({{
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname || "/mcp",
      method: "POST",
      headers: {{
        authorization: `Bearer ${{runtimeToken}}`,
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      }},
    }}, (res) => {{
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {{
        const text = Buffer.concat(chunks).toString("utf8");
        if (res.statusCode !== 200) return reject(new Error(`runtime MCP HTTP ${{res.statusCode}}: ${{text}}`));
        const response = JSON.parse(text);
        if (response.error) return reject(new Error(JSON.stringify(response.error)));
        if (response.result && response.result.isError) return reject(new Error(JSON.stringify(response.result)));
        resolve(response.result);
      }});
    }});
    req.setTimeout(3000, () => {{
      req.destroy(new Error(`runtime MCP ${{name}} timeout`));
    }});
    req.on("error", reject);
    req.end(body);
  }});
}}

async function emitForDeliveryToken(deliveryToken) {{
  if (deliveryToken) {{
    await callRuntimeTool("ack_workflow_turn", {{
      delivery_token: deliveryToken,
    }});
  }}
  if (intermediateOutput !== null) {{
    await callRuntimeTool("validate_and_submit_intermediate_workflow_run_output", {{
      workflow_output_json: JSON.stringify(intermediateOutput),
      ...(deliveryToken ? {{ delivery_token: deliveryToken }} : {{}}),
    }});
  }}
  process.stdout.write("```json\n" + JSON.stringify(finalPayload) + "\n```\n");
}}

async function emitOnce() {{
  const tokenMatches = [...promptBuffer.matchAll(/workflow-ack:[A-Za-z0-9_-]+/g)];
  const deliveryToken = tokenMatches.length ? tokenMatches[tokenMatches.length - 1][0] : undefined;
  if (!deliveryToken) {{
    if (emittedWithoutToken) return;
    if (!fallbackTimer) {{
      fallbackTimer = setTimeout(() => {{
        fallbackTimer = null;
        if (emittedWithoutToken) return;
        emittedWithoutToken = true;
        emitForDeliveryToken(undefined).catch((error) => {{
          process.stdout.write("dev-stub intermediate error: " + error.message + "\n");
        }});
      }}, 250);
    }}
    return;
  }}
  if (fallbackTimer) {{
    clearTimeout(fallbackTimer);
    fallbackTimer = null;
  }}
  if (emittedDeliveryTokens.has(deliveryToken)) return;
  emittedDeliveryTokens.add(deliveryToken);
  emitForDeliveryToken(deliveryToken).catch((error) => {{
    process.stdout.write("dev-stub intermediate error: " + error.message + "\n");
  }});
}}

process.stdin.setEncoding("utf8");
if (process.stdin.isTTY && typeof process.stdin.setRawMode === "function") {{
  process.stdin.setRawMode(true);
}}
process.stdin.on("data", (chunk) => {{
  promptBuffer += chunk;
  emitOnce().catch((error) => {{
    process.stdout.write("dev-stub intermediate error: " + error.message + "\n");
  }});
}});
process.stdin.resume();
"#,
        payload_json = payload,
        intermediate_json = intermediate_output,
    );
    format!(
        "stty -echo 2>/dev/null || true; node -e {}",
        shell_single_quote(&js)
    )
}

fn dev_stub_workflow_final_passthrough_script() -> String {
    let js = r#"const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");
const runtimeUrl = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_URL;
const runtimeToken = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN;
const emittedDeliveryTokens = new Set();
let emittedWithoutToken = false;
let promptBuffer = "";
let pendingTimer = null;

function callRuntimeTool(name, args) {
  return new Promise((resolve, reject) => {
    if (!runtimeUrl || !runtimeToken) return reject(new Error("runtime MCP binding missing"));
    const parsed = new URL(runtimeUrl);
    const body = JSON.stringify({
      jsonrpc: "2.0",
      id: `dev-stub-${Date.now()}`,
      method: "tools/call",
      params: { name, arguments: args },
    });
    const req = http.request({
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname || "/mcp",
      method: "POST",
      headers: {
        authorization: `Bearer ${runtimeToken}`,
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      },
    }, (res) => {
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        if (res.statusCode !== 200) return reject(new Error(`runtime MCP HTTP ${res.statusCode}: ${text}`));
        const response = JSON.parse(text);
        if (response.error) return reject(new Error(JSON.stringify(response.error)));
        if (response.result && response.result.isError) return reject(new Error(JSON.stringify(response.result)));
        resolve(response.result);
      });
    });
    req.setTimeout(3000, () => {
      req.destroy(new Error(`runtime MCP ${name} timeout`));
    });
    req.on("error", reject);
    req.end(body);
  });
}

function htmlOutputFromMessage(message) {
  if (!message) return null;
  if (message.kind === "html" && typeof message.html === "string") {
    return message;
  }
  if (message.kind === "html_file" && typeof message.path === "string") {
    const workspaceRoot = process.cwd();
    const absolutePath = path.resolve(workspaceRoot, message.path);
    const relativePath = path.relative(workspaceRoot, absolutePath);
    if (relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
      throw new Error(`upstream html_file path escapes workspace: ${message.path}`);
    }
    const html = fs.readFileSync(absolutePath, "utf8");
    return { kind: "html", html };
  }
  return null;
}

function parseMaybeJson(value) {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function extractHtmlOutputFromPrompt() {
  const matches = [...promptBuffer.matchAll(/```json\s*([\s\S]*?)```/g)];
  for (let index = matches.length - 1; index >= 0; index -= 1) {
    try {
      const parsed = JSON.parse(matches[index][1]);
      const message = parsed && parsed.output && parsed.output.message ? parsed.output.message : parsed;
      const output = htmlOutputFromMessage(parseMaybeJson(message));
      if (output) return { output };
    } catch {
      // Keep scanning older fenced JSON blocks.
    }
  }
  const rawPathMatch = promptBuffer.match(/"kind"\s*:\s*"html_file"[\s\S]*?"path"\s*:\s*"([^"]+)"/);
  if (rawPathMatch) {
    const output = htmlOutputFromMessage({ kind: "html_file", path: rawPathMatch[1] });
    if (output) return { output };
  }
  return null;
}

async function readHtmlOutput(deliveryToken) {
  const promptOutput = extractHtmlOutputFromPrompt();
  if (promptOutput) return promptOutput;
  const result = await callRuntimeTool("read_workflow_turn_context", {
    ...(deliveryToken ? { delivery_token: deliveryToken } : {}),
  });
  const context = result && (result.structuredContent || result.payload || result);
  const contextDeliveryToken = context && typeof context.delivery_token === "string" ? context.delivery_token : undefined;
  const messages = Array.isArray(context && context.messages) ? context.messages : [];
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const item = messages[index];
    const payload = item.parsed_handoff_payload || parseMaybeJson(item.handoff_payload);
    const completionMessage = payload && payload.completion && payload.completion.output
      ? payload.completion.output.message
      : undefined;
    const outputMessage = payload && payload.output ? payload.output.message : undefined;
    const output = htmlOutputFromMessage(parseMaybeJson(completionMessage ?? outputMessage ?? payload));
    if (output) return { output, deliveryToken: deliveryToken || contextDeliveryToken };
  }
  return null;
}

async function emitForDeliveryToken(deliveryToken) {
  const resolved = await readHtmlOutput(deliveryToken);
  if (!resolved || !resolved.output) throw new Error("no upstream html workflow output found");
  const output = resolved.output;
  const turnDeliveryToken = resolved.deliveryToken || deliveryToken;
  if (turnDeliveryToken) {
    await callRuntimeTool("ack_workflow_turn", {
      delivery_token: turnDeliveryToken,
    });
  }
  await callRuntimeTool("validate_and_submit_workflow_run_output", {
    workflow_output_json: JSON.stringify(output),
    ...(turnDeliveryToken ? { delivery_token: turnDeliveryToken } : {}),
  });
  const finalPayload = {
    summary: "passthrough html final output",
    output: { message: output },
  };
  process.stdout.write("```json\n" + JSON.stringify(finalPayload) + "\n```\n");
}

function scheduleEmit() {
  if (pendingTimer) return;
  pendingTimer = setTimeout(() => {
    pendingTimer = null;
    emitOnce().catch((error) => {
      if (String(error && error.message || error).includes("no active workflow turn for authenticated provider run")) {
        return;
      }
      process.stdout.write("dev-stub final passthrough error: " + error.message + "\n");
    });
  }, 100);
}

async function emitOnce() {
  const tokenMatches = [...promptBuffer.matchAll(/workflow-ack:[A-Za-z0-9_-]+/g)];
  const deliveryToken = tokenMatches.length ? tokenMatches[tokenMatches.length - 1][0] : undefined;
  if (!deliveryToken) {
    if (emittedWithoutToken) return;
    emittedWithoutToken = true;
    try {
      await emitForDeliveryToken(undefined);
    } catch (error) {
      emittedWithoutToken = false;
      throw error;
    }
    return;
  }
  if (emittedDeliveryTokens.has(deliveryToken)) return;
  emittedDeliveryTokens.add(deliveryToken);
  try {
    await emitForDeliveryToken(deliveryToken);
  } catch (error) {
    emittedDeliveryTokens.delete(deliveryToken);
    throw error;
  }
}

process.stdin.setEncoding("utf8");
if (process.stdin.isTTY && typeof process.stdin.setRawMode === "function") {
  process.stdin.setRawMode(true);
}
process.stdin.on("data", (chunk) => {
  promptBuffer += chunk;
  scheduleEmit();
});
setInterval(scheduleEmit, 500);
scheduleEmit();
process.stdin.resume();
"#;
    format!(
        "stty -echo 2>/dev/null || true; node -e {}",
        shell_single_quote(js)
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn dev_stub_semantic_renderer_script() -> String {
    let response = serde_json::json!({
        "kind": "http_response",
        "status": 200,
        "headers": { "content-type": "text/html; charset=utf-8" },
        "body": "<!doctype html><html><head><title>About Arroba Foods - Neon</title><style>body{background:#000;color:#39ff14;font-family:system-ui,sans-serif}a{color:#39ff14}main{border:1px solid #39ff14;padding:2rem;box-shadow:0 0 24px #39ff14}</style></head><body data-arroba-render=\"ARROBA_RENDER_NEON_GREEN\"><main><h1>About Arroba Foods</h1><p>We build practical grocery tools for neighborhood stores.</p><a href=\"/contact\">Contact us</a></main></body></html>"
    });
    let payload = serde_json::json!({
        "summary": "semantic renderer stub",
        "output": { "message": response }
    });
    format!(
        "while true; do sleep 2; printf '%s\\n%s\\n%s\\n' '```json' '{}' '```'; done",
        payload
    )
}

fn dev_stub_slow_first_output_script() -> String {
    let delay_ms = std::env::var("ARROBA_DEV_STUB_FIRST_OUTPUT_DELAY_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(35_000)
        .min(300_000);
    let delay_seconds = delay_ms as f64 / 1000.0;
    format!(
        "stty -echo 2>/dev/null || true; while IFS= read -r _line; do sleep {delay_seconds}; printf 'slow-first-output-drill complete\\n'; done"
    )
}

fn dev_stub_large_output_script() -> String {
    let line_count = std::env::var("ARROBA_DEV_STUB_LARGE_OUTPUT_LINES")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(1_000)
        .min(100_000);
    let line_bytes = std::env::var("ARROBA_DEV_STUB_LARGE_OUTPUT_LINE_BYTES")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(128)
        .clamp(32, 4096);
    let payload_bytes = line_bytes.saturating_sub(31);
    format!(
        "stty -echo 2>/dev/null || true; while IFS= read -r _line; do i=1; while [ \"$i\" -le {line_count} ]; do printf 'large-output-drill %06d ' \"$i\"; printf '%*s' {payload_bytes} '' | tr ' ' x; printf '\\n'; i=$((i + 1)); done; done"
    )
}
