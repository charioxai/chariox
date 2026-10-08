pub(super) const NODE_WORKFLOW_CODE_COMPILER: &str = r#"
import vm from "node:vm"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const input = JSON.parse(Buffer.concat(chunks).toString() || "{}")

function createBuilder() {
  let nextSchema = 1
  let nextNode = 1
  let nextEdge = 1
  let nextEndpoint = 1
  let nextQueue = 1
  let nextSchedule = 1
  const sourceSpans = {}
  const state = {
    schema_version: 1,
    parameters_schema: undefined,
    workflow: {},
    schemas: [],
    nodes: [],
    edges: [],
    endpoints: [],
    queues: [],
    schedules: []
  }
  function handle(kind, explicit) {
    return explicit || `${kind}:${kind === "schema" ? nextSchema++ : kind === "node" ? nextNode++ : kind === "edge" ? nextEdge++ : kind === "endpoint" ? nextEndpoint++ : kind === "queue" ? nextQueue++ : nextSchedule++}`
  }
  function sourceSpan() {
    const stack = String(new Error().stack || "")
    const frame = stack
      .split("\n")
      .map((line) => line.match(/workflow-code\.js:(\d+):(\d+)/))
      .find(Boolean)
    if (!frame) return undefined
    const line = Math.max(1, Number(frame[1]) - 1)
    const column = Math.max(1, Number(frame[2]))
    return { start_line: line, start_column: column, end_line: line, end_column: column }
  }
  function recordSourceSpan(handle) {
    const span = sourceSpan()
    if (span) sourceSpans[handle] = span
  }
  function isPlainObject(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value)
  }
  function own(value, key) {
    return Object.prototype.hasOwnProperty.call(value, key)
  }
  function validateParameterSchema(schema, pathLabel = "parameters schema") {
    if (!isPlainObject(schema)) throw new Error(`${pathLabel} must be an object`)
    const supported = new Set([
      "type",
      "properties",
      "required",
      "default",
      "minimum",
      "maximum",
      "multipleOf",
      "title",
      "description",
      "enum",
      "additionalProperties",
      "xPowerOfTwo"
    ])
    for (const key of Object.keys(schema)) {
      if (!supported.has(key)) throw new Error(`${pathLabel}.${key} is not supported`)
    }
    const type = schema.type
    if (type !== undefined && !["object", "string", "number", "integer", "boolean"].includes(type)) {
      throw new Error(`${pathLabel}.type is not supported`)
    }
    if (schema.enum !== undefined && !Array.isArray(schema.enum)) {
      throw new Error(`${pathLabel}.enum must be an array`)
    }
    if (schema.required !== undefined && (!Array.isArray(schema.required) || schema.required.some((item) => typeof item !== "string"))) {
      throw new Error(`${pathLabel}.required must be an array of strings`)
    }
    if (schema.additionalProperties !== undefined && typeof schema.additionalProperties !== "boolean") {
      throw new Error(`${pathLabel}.additionalProperties must be a boolean`)
    }
    if (schema.xPowerOfTwo !== undefined && typeof schema.xPowerOfTwo !== "boolean") {
      throw new Error(`${pathLabel}.xPowerOfTwo must be a boolean`)
    }
    for (const numberKey of ["minimum", "maximum", "multipleOf"]) {
      if (schema[numberKey] !== undefined && typeof schema[numberKey] !== "number") {
        throw new Error(`${pathLabel}.${numberKey} must be a number`)
      }
    }
    if (schema.properties !== undefined) {
      if (!isPlainObject(schema.properties)) throw new Error(`${pathLabel}.properties must be an object`)
      for (const [key, child] of Object.entries(schema.properties)) {
        validateParameterSchema(child, `${pathLabel}.properties.${key}`)
      }
    }
  }
  function validateParameterValue(name, value, schema) {
    const type = schema.type
    if (schema.enum !== undefined && !schema.enum.some((item) => Object.is(item, value))) {
      throw new Error(`parameter ${name} must be one of the declared enum values`)
    }
    if (type === "string" && typeof value !== "string") throw new Error(`parameter ${name} must be a string`)
    if (type === "boolean" && typeof value !== "boolean") throw new Error(`parameter ${name} must be a boolean`)
    if (type === "number" && (typeof value !== "number" || !Number.isFinite(value))) throw new Error(`parameter ${name} must be a number`)
    if (type === "integer" && (!Number.isSafeInteger(value))) throw new Error(`parameter ${name} must be an integer`)
    if ((type === "number" || type === "integer") && schema.minimum !== undefined && value < schema.minimum) {
      throw new Error(`parameter ${name} must be >= ${schema.minimum}`)
    }
    if ((type === "number" || type === "integer") && schema.maximum !== undefined && value > schema.maximum) {
      throw new Error(`parameter ${name} must be <= ${schema.maximum}`)
    }
    if ((type === "number" || type === "integer") && schema.multipleOf !== undefined && value % schema.multipleOf !== 0) {
      throw new Error(`parameter ${name} must be a multiple of ${schema.multipleOf}`)
    }
    if (schema.xPowerOfTwo === true) {
      if (!Number.isSafeInteger(value) || value < 1 || Math.log2(value) % 1 !== 0) {
        throw new Error(`parameter ${name} must be a power of two`)
      }
    }
  }
  function resolveParameters(schema) {
    validateParameterSchema(schema)
    if (schema.type !== "object") throw new Error("parameters schema.type must be object")
    const supplied = input.parameters === undefined ? {} : input.parameters
    if (!isPlainObject(supplied)) throw new Error("workflow-code parameters must be an object")
    const properties = schema.properties || {}
    const required = new Set(schema.required || [])
    const resolved = {}
    for (const key of Object.keys(supplied)) {
      if (!own(properties, key) && schema.additionalProperties === false) {
        throw new Error(`parameter ${key} is not declared by the schema`)
      }
    }
    for (const [key, propertySchema] of Object.entries(properties)) {
      if (own(supplied, key)) {
        validateParameterValue(key, supplied[key], propertySchema)
        resolved[key] = supplied[key]
      } else if (own(propertySchema, "default")) {
        validateParameterValue(key, propertySchema.default, propertySchema)
        resolved[key] = propertySchema.default
      } else if (required.has(key)) {
        throw new Error(`parameter ${key} is required`)
      }
    }
    if (schema.additionalProperties !== false) {
      for (const [key, value] of Object.entries(supplied)) {
        if (!own(resolved, key)) resolved[key] = value
      }
    }
    return Object.freeze(resolved)
  }
  function loadSchemaFromFile(schemaPath) {
    if (typeof schemaPath !== "string" || schemaPath.trim() === "") {
      throw new Error("schemaFromFile path must be a non-empty string")
    }
    if (!input.schema_files) {
      throw new Error("schemaFromFile requires an approved schema import root")
    }
    if (schemaPath.startsWith("/") || schemaPath.split(/[\\/]/).includes("..")) {
      throw new Error("schemaFromFile path must stay inside the approved import root")
    }
    if (!schemaPath.endsWith(".json")) throw new Error("schemaFromFile path must end in .json")
    const key = schemaPath.replace(/\\/g, "/").split("/").filter(part => part !== "." && part !== "").join("/")
    if (!Object.prototype.hasOwnProperty.call(input.schema_files, key)) {
      if (Object.prototype.hasOwnProperty.call(input.schema_errors || {}, key)) throw new Error(input.schema_errors[key])
      schemaRequests.add(key)
      throw new Error("schemaFromFile is pending approved data")
    }
    return JSON.parse(input.schema_files[key])
  }

  function ref(value, expected) {
    if (typeof value === "string") return value
    if (value && value.__workflowCodeHandle === expected) return value.handle
    throw new Error(`expected ${expected} handle`)
  }
  function agent(value) {
    if (value && value.__workflowCodeAgent) return value.binding
    if (value && typeof value === "object" && typeof value.kind === "string") return value
    throw new Error("node agent must be created with workflow.newAgent or workflow.existingAgent")
  }
  const api = {
    parameters(options = {}) {
      const schema = options.schema
      if (state.parameters_schema !== undefined) {
        throw new Error("workflow.parameters may only be called once")
      }
      state.parameters_schema = schema
      return resolveParameters(schema)
    },
    define(options = {}) {
      state.workflow = {
        ...state.workflow,
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.prompt !== undefined ? { prompt: options.prompt } : {}),
        ...(options.flushAgentContextBeforeRun !== undefined ? { flush_agent_context_before_run: options.flushAgentContextBeforeRun } : {}),
        ...(options.maxConcurrent !== undefined ? { max_concurrent: options.maxConcurrent } : {}),
        ...(options.runOutputSchema !== undefined ? { run_output_schema: ref(options.runOutputSchema, "schema") } : {})
      }
      return api
    },
    schema(options = {}) {
      const item = {
        handle: handle("schema", options.handle),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.description !== undefined ? { description: options.description } : {}),
        schema: options.schema
      }
      recordSourceSpan(item.handle)
      state.schemas.push(item)
      return { __workflowCodeHandle: "schema", handle: item.handle }
    },
    schemaFromFile(pathOrOptions, maybeOptions = {}) {
      const options = typeof pathOrOptions === "string"
        ? { ...maybeOptions, path: pathOrOptions }
        : { ...(pathOrOptions || {}) }
      const item = {
        handle: handle("schema", options.handle),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.description !== undefined ? { description: options.description } : {}),
        schema: loadSchemaFromFile(options.path)
      }
      recordSourceSpan(item.handle)
      state.schemas.push(item)
      return { __workflowCodeHandle: "schema", handle: item.handle }
    },
    newAgent(options = {}) {
      return {
        __workflowCodeAgent: true,
        binding: {
          kind: "create",
          ...(options.alias !== undefined ? { alias: options.alias } : {}),
          provider: options.provider,
          ...(options.model !== undefined ? { model: options.model } : {}),
          ...(options.effort !== undefined ? { effort: options.effort } : {}),
          ...(options.accountProfile !== undefined ? { account_profile: options.accountProfile } : {})
        }
      }
    },
    existingAgent(agentRef) {
      return {
        __workflowCodeAgent: true,
        binding: { kind: "existing", agent_ref: agentRef }
      }
    },
    node(options = {}) {
      const item = {
        handle: handle("node", options.handle),
        agent: agent(options.agent),
        ...(options.publicLabel !== undefined ? { public_label: options.publicLabel } : {}),
        ...(options.instructions !== undefined ? { instructions: options.instructions } : {}),
        ...(options.canCompleteWorkflowRun !== undefined ? { can_complete_workflow_run: options.canCompleteWorkflowRun } : {}),
        ...(options.canEmitIntermediateRunOutput !== undefined ? { can_emit_intermediate_run_output: options.canEmitIntermediateRunOutput } : {}),
        ...(options.waitForAllInputs !== undefined ? { wait_for_all_inputs: options.waitForAllInputs } : {}),
        ...(options.intermediateOutputSchema !== undefined ? { intermediate_output_schema: ref(options.intermediateOutputSchema, "schema") } : {}),
        ...(options.maxTurns !== undefined ? { max_turns: options.maxTurns } : {}),
        ...(options.extensions !== undefined ? { extensions: options.extensions } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.nodes.push(item)
      return { __workflowCodeHandle: "node", handle: item.handle }
    },
    edge(fromNode, toNode, options = {}) {
      const item = {
        handle: handle("edge", options.handle),
        from_node: ref(fromNode, "node"),
        to_node: ref(toNode, "node"),
        ...(options.sourceSide !== undefined ? { source_side: options.sourceSide } : {}),
        ...(options.targetSide !== undefined ? { target_side: options.targetSide } : {}),
        ...(options.handoffSchema !== undefined ? { handoff_schema: ref(options.handoffSchema, "schema") } : {}),
        ...(options.validationPolicy !== undefined ? { validation_policy: options.validationPolicy } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.edges.push(item)
      return { __workflowCodeHandle: "edge", handle: item.handle }
    },
    endpoint(entryNode, options = {}) {
      const item = {
        handle: handle("endpoint", options.handle),
        entry_node: ref(entryNode, "node"),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.maxInstances !== undefined ? { max_instances: options.maxInstances } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.endpoints.push(item)
      return { __workflowCodeHandle: "endpoint", handle: item.handle }
    },
    queue(options = {}) {
      const item = {
        handle: handle("queue", options.handle),
        alias: options.alias || "default",
        ...(options.priority !== undefined ? { priority: options.priority } : {}),
        ...(options.enabled !== undefined ? { enabled: options.enabled } : {})
      }
      recordSourceSpan(item.handle)
      state.queues.push(item)
      return { __workflowCodeHandle: "queue", handle: item.handle }
    },
    schedule(endpoint, options = {}) {
      const trigger = options.trigger !== undefined
        ? options.trigger
        : options.cron !== undefined
          ? { kind: "cron", expression: options.cron, timezone: options.timezone || options.tz || "UTC" }
          : { kind: "interval", every_seconds: options.everySeconds ?? options.intervalSeconds }
      const item = {
        handle: handle("schedule", options.handle),
        endpoint: ref(endpoint, "endpoint"),
        ...(options.queue !== undefined ? { queue: ref(options.queue, "queue") } : {}),
        ...(options.enabled !== undefined ? { enabled: options.enabled } : {}),
        trigger,
        invocation_prompt: options.invocationPrompt,
        overlap_policy: options.overlapPolicy ?? options.overlap ?? options.policy,
        ...(options.maxRuns !== undefined || options.maxWakeups !== undefined ? { max_runs: options.maxRuns ?? options.maxWakeups } : {})
      }
      recordSourceSpan(item.handle)
      state.schedules.push(item)
      return { __workflowCodeHandle: "schedule", handle: item.handle }
    },
    watchdog(endpoint, options = {}) {
      const item = this.schedule(endpoint, {
        ...options,
        handle: options.handle || handle("watchdog", undefined),
        intervalSeconds: options.intervalSeconds,
        invocationPrompt: options.invocationPrompt,
        policy: options.policy,
        maxRuns: options.maxWakeups
      })
      return { __workflowCodeHandle: "watchdog", handle: item.handle }
    },
    export() {
      return {
        ...state,
        ...(state.parameters_schema !== undefined ? { parameters_schema: state.parameters_schema } : {})
      }
    },
    __sourceSpans() {
      return sourceSpans
    }
  }
  return api
}

try {
  let source = String(input.source || "")
  const stripper = input.typescript_stripper
  delete input.typescript_stripper
  if (input.language === "typescript") {
    // Same stripper and options as official Node's
    // stripTypeScriptTypes(source, {mode: "transform"}), independent of
    // whether this Node build ships it. Only the source text crosses in.
    const amaro = {exports: {}}
    const builtins = {util: await import("node:util"), "node:buffer": await import("node:buffer")}
    vm.compileFunction(String(stripper), ["module", "exports", "require"], {filename: "amaro.js"})(
      amaro, amaro.exports, name => builtins[name])
    source = amaro.exports.transformSync(source, {mode: "transform", sourceMap: false, filename: ""}).code
  }
  // MP-08/MP-11: create all objects/functions in the evaluated realm. The
  // boundary consists solely of JSON strings; never inject a host callback.
  if (vm.constants?.DONT_CONTEXTIFY === undefined) {
    throw new Error("isolated workflow-code compilation requires Node.js with isolated realm support")
  }
  const context = vm.createContext(vm.constants.DONT_CONTEXTIFY, {
    codeGeneration: { strings: false, wasm: false },
    microtaskMode: "afterEvaluate"
  })
  const bootstrap = `const input = JSON.parse(${JSON.stringify(JSON.stringify(input))});
    const schemaRequests = new Set();
    const createBuilder = ${createBuilder.toString()};
    const workflow = createBuilder();
    const console = {log() {}, error() {}};
    globalThis.workflow = workflow; globalThis.console = console;`
  new vm.Script(bootstrap).runInContext(context, {timeout: input.timeout_ms})
  const wrapped = `(async () => {\n${source}\nif (typeof defineWorkflow === "function") await defineWorkflow(workflow)\nif (schemaRequests.size) return JSON.stringify({ok: false, schema_requests: Array.from(schemaRequests)})\nreturn JSON.stringify({ok: true, definition: workflow.export(), source_spans: workflow.__sourceSpans(), logs: ""})\n})()`
  // Promise settlement stays in the realm too: awaiting a realm thenable in
  // the host would pass host resolve/reject functions back to user code.
  const script = new vm.Script(`${wrapped}.then(
    value => {globalThis.compilerResult = value},
    error => {globalThis.compilerResult = JSON.stringify({ok: false, schema_requests: Array.from(schemaRequests), error: String(error && error.message ? error.message : error), logs: ""})}
  ); void 0`, { filename: "workflow-code.js" })
  const timeout = Math.max(1, Number(input.timeout_ms || 30000))
  script.runInContext(context, { timeout })
  const readResult = new vm.Script('typeof compilerResult === "string" ? compilerResult : undefined')
  const deadline = Date.now() + timeout
  let result
  while ((result = readResult.runInContext(context, {timeout})) === undefined) {
    if (Date.now() >= deadline) throw new Error("workflow-code script exceeded configured timeout")
    await new Promise(resolve => setTimeout(resolve, 5))
  }
  process.stdout.write(result)

} catch (error) {
  console.log(JSON.stringify({
    ok: false,
    error: String(error && error.message ? error.message : error),
    logs: ""
  }))
}
"#;
