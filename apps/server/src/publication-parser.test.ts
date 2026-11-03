import assert from "node:assert/strict"
import test from "node:test"
import { parseAndValidateRequest } from "./publication-parser.js"
import type { GatewayRequest, WorkflowPublicationConfig } from "./publication-types.js"
const publication = { publication_id:"p",session_id:"s",workflow_ref:"w",endpoint_ref:"e",input_schema: { type: "object", properties: { count: { type: "integer" }, urgent: { type: "boolean" } }, required: ["count", "urgent"], additionalProperties: false } } as WorkflowPublicationConfig
const request = (url: string) => ({ method: "GET", url, headers: {} }) as GatewayRequest

test("typed GET preserves prompt and query lexemes for kernel coercion", async () => {
  const data = await parseAndValidateRequest(request("/?prompt=triage&count=7&urgent=true"), publication)
  assert.deepEqual({ ...(data as object) }, { prompt: "triage", count: "7", urgent: "true" })
})
test("typed GET refuses ambiguous, unsafe and undeclared data", async () => {
  for (const query of ["count=01&urgent=true", "count=9007199254740992&urgent=true", "count=7&urgent=1", "count=7&count=8&urgent=true", "count=7&urgent=true&extra=x"]) {
    await assert.rejects(parseAndValidateRequest(request(`/?prompt=triage&${query}`), publication))
  }
})
test("oversized GET tells the caller to send POST", async () => {
  await assert.rejects(parseAndValidateRequest(request(`/?prompt=${"x".repeat(8192)}`), publication), /send POST/)
})

test("HTTP gate acceptance exposes status without owner replay state", async () => {
  const {forwardWorkflowResult}=await import("./publication-http-response.js")
  let status=0
  const result=forwardWorkflowResult({code:value=>{status=value},headers:()=>{}},publication,{accepted:true,gate_entry:{entry_id:"e",disposition:"ready",next_due_at_ms:1,deadline_ms:10,decisions:[{private:"private-business-state"}]} as unknown as import("@chariox/kernel-client").WorkflowGateEntry})
  assert.equal(status,202)
  assert.deepEqual(result,{accepted:true,gate_entry:{entry_id:"e",disposition:"ready",next_due_at_ms:1,deadline_ms:10}})
  assert.ok(!JSON.stringify(result).includes("private-business-state"))
})
