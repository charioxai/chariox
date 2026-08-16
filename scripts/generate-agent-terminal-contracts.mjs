#!/usr/bin/env node
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const repositoryRoot = path.resolve(scriptDirectory, "..")
const kernelSourceRoot = path.join(repositoryRoot, "apps", "kernel", "src")
const requestPath = path.join(kernelSourceRoot, "local", "api", "types", "request.rs")
const manifestPath = path.join(kernelSourceRoot, "runtime", "terminal_operation_registry", "parity_manifest.json")
const outputPath = path.join(kernelSourceRoot, "runtime", "terminal_operation_registry", "contracts.json")

async function rustFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...await rustFiles(entryPath))
    else if (entry.isFile() && entry.name.endsWith(".rs")) files.push(entryPath)
  }
  return files
}

function matchingBrace(source, openIndex) {
  let depth = 0
  let quote = null
  let escaped = false
  for (let index = openIndex; index < source.length; index += 1) {
    const character = source[index]
    if (quote) {
      if (escaped) escaped = false
      else if (character === "\\") escaped = true
      else if (character === quote) quote = null
      continue
    }
    if (character === '"') {
      quote = character
      continue
    }
    if (character === "{") depth += 1
    else if (character === "}" && --depth === 0) return index
  }
  throw new Error(`unclosed Rust brace at ${openIndex}`)
}

function splitTopLevel(source, separator = ",") {
  const chunks = []
  let start = 0
  let angle = 0
  let brace = 0
  let bracket = 0
  let paren = 0
  let quote = null
  let escaped = false
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]
    if (quote) {
      if (escaped) escaped = false
      else if (character === "\\") escaped = true
      else if (character === quote) quote = null
      continue
    }
    if (character === '"') {
      quote = character
      continue
    }
    if (character === "<") angle += 1
    else if (character === ">" && angle > 0) angle -= 1
    else if (character === "{") brace += 1
    else if (character === "}" && brace > 0) brace -= 1
    else if (character === "[") bracket += 1
    else if (character === "]" && bracket > 0) bracket -= 1
    else if (character === "(") paren += 1
    else if (character === ")" && paren > 0) paren -= 1
    else if (character === separator && angle === 0 && brace === 0 && bracket === 0 && paren === 0) {
      chunks.push(source.slice(start, index))
      start = index + 1
    }
  }
  chunks.push(source.slice(start))
  return chunks
}

function serdeAttributeText(attributes) {
  return attributes.map((entry) => entry[1]).join(",")
}

function parseFields(body, allowPrivate = false) {
  const fields = []
  for (const chunk of splitTopLevel(body)) {
    const trimmed = chunk.trim()
    const field = trimmed.match(allowPrivate
      ? /(?:^|\n)\s*(?:pub(?:\([^)]*\))?\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([\s\S]+)$/
      : /(?:^|\n)\s*pub(?:\([^)]*\))?\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([\s\S]+)$/)
    if (!field) continue
    const attributes = [...trimmed.matchAll(/#\[serde\(([^\]]+)\)\]/g)]
    const attributeText = serdeAttributeText(attributes)
    const rename = attributeText.match(/rename\s*=\s*"([^"]+)"/)
    fields.push({
      name: field[1],
      serdeName: rename?.[1] ?? field[1],
      type: field[2].trim(),
      optional: /^Option\s*</.test(field[2].trim()),
      defaulted: /(?:^|,)\s*default(?:\s*=|,|$)/.test(attributeText),
      skipped: /(?:^|,)\s*skip(?:_serializing|_deserializing)?(?:\s*=|,|$)/.test(attributeText),
      flattened: /(?:^|,)\s*flatten(?:\s*=|,|$)/.test(attributeText),
    })
  }
  return fields
}

function parseStructs(source, sourcePath) {
  const structs = new Map()
  const pattern = /pub\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>{}]*>)?\s*\{/g
  for (const match of source.matchAll(pattern)) {
    const openIndex = match.index + match[0].lastIndexOf("{")
    const body = source.slice(openIndex + 1, matchingBrace(source, openIndex))
    structs.set(match[1], { fields: parseFields(body), sourcePath })
  }
  return structs
}

function renameRustName(value, renameAll) {
  if (renameAll === "snake_case") return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase()
  if (renameAll === "kebab-case") return value.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase()
  if (renameAll === "SCREAMING_SNAKE_CASE") return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toUpperCase()
  return value
}

function parseEnums(source, sourcePath) {
  const enums = new Map()
  const pattern = /((?:#\[[^\]]+\]\s*)*)pub\s+enum\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>{}]*>)?\s*\{/g
  for (const match of source.matchAll(pattern)) {
    const openIndex = match.index + match[0].lastIndexOf("{")
    const body = source.slice(openIndex + 1, matchingBrace(source, openIndex))
    const attributes = [...match[1].matchAll(/#\[serde\(([^\]]+)\)\]/g)]
    const attributeText = serdeAttributeText(attributes)
    const tag = attributeText.match(/(?:^|,)\s*tag\s*=\s*"([^"]+)"/)?.[1] ?? null
    const renameAll = attributeText.match(/(?:^|,)\s*rename_all\s*=\s*"([^"]+)"/)?.[1] ?? null
    const variants = []
    for (const chunk of splitTopLevel(body)) {
      const trimmed = chunk.trim()
      const variantMatch = trimmed.match(/^(?:#\[[^\]]+\]\s*)*([A-Za-z_][A-Za-z0-9_]*)(?:\s*(?:\{([\s\S]*)\}|\(([\s\S]*)\)))?$/)
      if (!variantMatch) continue
      const variantAttributes = [...trimmed.matchAll(/#\[serde\(([^\]]+)\)\]/g)]
      const variantAttributeText = serdeAttributeText(variantAttributes)
      const rename = variantAttributeText.match(/rename\s*=\s*"([^"]+)"/)?.[1]
      variants.push({
        name: variantMatch[1],
        serdeName: rename ?? renameRustName(variantMatch[1], renameAll),
        fields: variantMatch[2] ? parseFields(variantMatch[2], true) : [],
        tupleType: variantMatch[3]?.trim() || null,
      })
    }
    enums.set(match[2], { tag, variants, sourcePath })
  }
  return enums
}

function parseTypeAliases(source) {
  const aliases = new Map()
  const pattern = /\bpub\s+type\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^;]+);/g
  for (const match of source.matchAll(pattern)) {
    aliases.set(match[1], match[2].trim())
  }
  return aliases
}

function parseRequests(source) {
  const start = source.indexOf("pub enum LocalDaemonRequest")
  if (start < 0) throw new Error("LocalDaemonRequest was not found")
  const openIndex = source.indexOf("{", start)
  const body = source.slice(openIndex + 1, matchingBrace(source, openIndex))
  const requests = new Map()
  for (const chunk of splitTopLevel(body)) {
    const match = chunk.trim().match(/^([A-Za-z_][A-Za-z0-9_]*)\s*\(\s*([\s\S]+?)\s*\)$/)
    if (match) requests.set(match[1], match[2].replace(/^Box\s*</, "").replace(/>$/, "").trim())
  }
  return requests
}

function unwrapType(type) {
  let value = type.trim()
  let optional = false
  if (/^Option\s*</.test(value)) {
    optional = true
    value = value.slice(value.indexOf("<") + 1, value.lastIndexOf(">"))
  }
  return { type: value.trim(), optional }
}

function schemaForType(type, structs, enums, aliases, stack = []) {
  const unwrapped = unwrapType(type)
  if (unwrapped.optional) {
    return { oneOf: [schemaForType(unwrapped.type, structs, enums, aliases, stack), { type: "null" }] }
  }
  const baseType = unwrapped.type.split("::").at(-1)
  const aliasTarget = aliases.get(baseType)
  if (aliasTarget && !stack.includes(baseType)) {
    return schemaForType(aliasTarget, structs, enums, aliases, [...stack, baseType])
  }
  let schema
  if (/^Box\s*</.test(unwrapped.type)) {
    schema = schemaForType(unwrapped.type.slice(unwrapped.type.indexOf("<") + 1, unwrapped.type.lastIndexOf(">")), structs, enums, aliases, stack)
  } else if (/^(?:Vec|HashSet|SmallVec)\s*</.test(unwrapped.type)) {
    const inner = unwrapped.type.slice(unwrapped.type.indexOf("<") + 1, unwrapped.type.lastIndexOf(">"))
    schema = { type: "array", items: schemaForType(inner, structs, enums, aliases, stack) }
  } else if (/^(?:String|&str|PathBuf|Url|Uuid|[A-Za-z_][A-Za-z0-9_]*Id)$/.test(unwrapped.type)) schema = { type: "string" }
  else if (/^(?:bool)$/.test(unwrapped.type)) schema = { type: "boolean" }
  else if (/^(?:f)(?:32|64)?$/.test(unwrapped.type)) schema = { type: "number" }
  else if (/^(?:u|i)(?:8|16|32|64|128|size)?$/.test(unwrapped.type) || /(?:u|i)(?:8|16|32|64|128|size)$/.test(unwrapped.type)) schema = { type: "integer" }
  else if (/^(?:\(\))$/.test(unwrapped.type)) schema = { type: "null" }
  else if (/^(?:serde_json::)?Value$/.test(unwrapped.type)) schema = {}
  else if (/^(?:HashMap|BTreeMap)</.test(unwrapped.type)) {
    const inner = unwrapped.type.slice(unwrapped.type.indexOf("<") + 1, unwrapped.type.lastIndexOf(">"))
    const valueType = splitTopLevel(inner)[1] ?? "serde_json::Value"
    schema = { type: "object", additionalProperties: schemaForType(valueType, structs, enums, aliases, stack) }
  } else {
    const enumDefinition = enums.get(baseType)
    if (enumDefinition) schema = schemaForEnum(enumDefinition, structs, enums, aliases, stack)
    const structDefinition = structs.get(baseType)
    if (!schema && structDefinition && !stack.includes(baseType)) {
    const properties = {}
    const required = []
    for (const field of structDefinition.fields) {
      if (field.skipped) continue
      if (field.flattened) return { type: "object" }
      properties[field.serdeName] = schemaForType(field.type, structs, enums, aliases, [...stack, baseType])
      if (!field.optional && !field.defaulted) required.push(field.serdeName)
    }
      schema = { type: "object", additionalProperties: false, properties, ...(required.length ? { required } : {}) }
    }
  }
  const resolved = schema ?? { type: "object" }
  return resolved
}

function schemaForEnum(definition, structs, enums, aliases, stack) {
  if (!definition.tag && definition.variants.every((variant) => !variant.fields.length && !variant.tupleType)) {
    return { type: "string", enum: definition.variants.map((variant) => variant.serdeName) }
  }
  if (!definition.tag) {
    return {
      oneOf: definition.variants.map((variant) => {
        if (variant.tupleType) {
          const tupleTypes = splitTopLevel(variant.tupleType).map((entry) => entry.trim()).filter(Boolean)
          const value = tupleTypes.length === 1
            ? schemaForType(tupleTypes[0], structs, enums, aliases, [...stack, variant.name])
            : { type: "array", items: tupleTypes.map((entry) => schemaForType(entry, structs, enums, aliases, [...stack, variant.name])) }
          return { type: "object", additionalProperties: false, properties: { [variant.serdeName]: value }, required: [variant.serdeName] }
        }
        const properties = {}
        const required = []
        for (const field of variant.fields) {
          if (field.skipped) continue
          properties[field.serdeName] = schemaForType(field.type, structs, enums, aliases, [...stack, variant.name])
          if (!field.optional && !field.defaulted) required.push(field.serdeName)
        }
        return {
          type: "object",
          additionalProperties: false,
          properties: { [variant.serdeName]: { type: "object", additionalProperties: false, properties, ...(required.length ? { required } : {}) } },
          required: [variant.serdeName],
        }
      }),
    }
  }
  return {
    oneOf: definition.variants.map((variant) => {
      const properties = { [definition.tag]: { type: "string", const: variant.serdeName } }
      const required = [definition.tag]
      for (const field of variant.fields) {
        if (field.skipped) continue
        properties[field.serdeName] = schemaForType(field.type, structs, enums, aliases, [...stack, definition])
        if (!field.optional && !field.defaulted) required.push(field.serdeName)
      }
      return { type: "object", additionalProperties: false, properties, required }
    }),
  }
}

const TARGET_FIELDS = new Set([
  "session_id", "attachment_id", "agent_id", "target_agent_id", "agent_ref",
  "workflow_id", "workflow_ref", "node_id", "workflow_run_id", "workflow_run_ref",
  "project_id", "workspace_id", "worktree_id", "slice_ref", "machine_id", "machine_ref",
  "provider_run_id", "prompt_id", "interaction_id", "publication_id", "endpoint_id",
  "connection_id", "binding_id", "credential_id", "connector_id", "skill_id", "script_id",
  "environment_id", "schedule_id", "watchdog_id", "queue_id", "event_id", "event_connection_id",
])

function contractFor(variant, requestType, structs, enums, aliases) {
  const definition = structs.get(requestType)
  const fields = definition?.fields ?? []
  const properties = {}
  const required = []
  const requiredTargets = []
  for (const field of fields) {
    properties[field.serdeName] = field.serdeName === "prompt_source"
      ? { type: "string", const: "agent_terminal" }
      : schemaForType(field.type, structs, enums, aliases)
    const explicitAgentTarget = field.serdeName === "target_agent_id" || field.serdeName === "agent_id"
    if ((!field.optional && !field.defaulted) || explicitAgentTarget) required.push(field.serdeName)
    if (TARGET_FIELDS.has(field.serdeName) && (explicitAgentTarget || (!field.optional && !field.defaulted))) requiredTargets.push(field.serdeName)
  }
  let inputSchema = fields.length === 0
    ? { type: "null" }
    : { type: "object", additionalProperties: false, properties, ...(required.length ? { required: [...new Set(required)] } : {}) }
  if (variant === "SubmitPrompts") {
    inputSchema = {
      type: "object",
      additionalProperties: false,
      required: ["session_id", "attachment_id", "prompts"],
      properties: {
        session_id: { type: "string" },
        attachment_id: { type: "string" },
        max_concurrency: { type: "number" },
        prompts: {
          type: "array",
          items: {
            type: "object",
            additionalProperties: false,
            required: ["target_agent_id", "prompt"],
            properties: {
              session_id: { type: "string" },
              attachment_id: { type: "string" },
              target_agent_id: { type: "string" },
              prompt: { type: "string" },
              attachments: { type: "array", items: { type: "object" } },
              prompt_source: { type: "string", const: "agent_terminal" },
            },
          },
        },
      },
    }
  }
  return { required_targets: [...new Set(requiredTargets)], input_schema: inputSchema, request_type: requestType, source: definition?.sourcePath ?? null }
}

const [requestSource, manifestSource, ...sources] = await Promise.all([
  readFile(requestPath, "utf8"),
  readFile(manifestPath, "utf8"),
  ...(await rustFiles(kernelSourceRoot)).filter((file) => file !== requestPath).map((file) => readFile(file, "utf8").then((text) => ({ text, file }))),
])
const structs = new Map()
const enums = new Map()
const aliases = new Map()
for (const source of sources) {
  for (const [name, definition] of parseStructs(source.text, path.relative(repositoryRoot, source.file))) structs.set(name, definition)
  for (const [name, definition] of parseEnums(source.text, path.relative(repositoryRoot, source.file))) enums.set(name, definition)
  for (const [name, target] of parseTypeAliases(source.text)) aliases.set(name, target)
}
const requests = parseRequests(requestSource)
const manifest = JSON.parse(manifestSource)
const contracts = {}
for (const entry of manifest.requests) {
  if (entry.classification !== "agent_terminal_supported") continue
  const requestType = requests.get(entry.variant)
  if (!requestType) throw new Error(`request type missing for ${entry.variant}`)
  contracts[entry.variant] = contractFor(entry.variant, requestType, structs, enums, aliases)
}
await mkdir(path.dirname(outputPath), { recursive: true })
await writeFile(outputPath, `${JSON.stringify({ schema_version: 1, source: "LocalDaemonRequest", contracts }, null, 2)}\n`, "utf8")
console.log(JSON.stringify({ ok: true, contracts: Object.keys(contracts).length, output: outputPath }))
