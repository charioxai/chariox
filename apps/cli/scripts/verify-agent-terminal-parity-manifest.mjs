#!/usr/bin/env node
import { createHash } from "node:crypto"
import { readFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptsDirectory = path.dirname(fileURLToPath(import.meta.url))
const repositoryRoot = path.resolve(scriptsDirectory, "../../..")
const requestPath = path.join(repositoryRoot, "apps/kernel/src/local/api/types/request.rs")
const manifestPath = path.join(repositoryRoot, "apps/kernel/src/runtime/terminal_operation_registry/parity_manifest.json")
const contractsPath = path.join(repositoryRoot, "apps/kernel/src/runtime/terminal_operation_registry/contracts.json")

const requestSource = await readFile(requestPath, "utf8")
const manifest = JSON.parse(await readFile(manifestPath, "utf8"))
const contracts = JSON.parse(await readFile(contractsPath, "utf8"))
const requestBody = requestSource.slice(requestSource.indexOf("pub enum LocalDaemonRequest"), requestSource.lastIndexOf("}"))
const requestVariants = [...requestBody.matchAll(/^\s{4}([A-Za-z0-9_]+)\(/gm)].map((match) => match[1])
const entries = Array.isArray(manifest.requests) ? manifest.requests : []
const manifestVariants = entries.map((entry) => entry?.variant)
const duplicateVariants = manifestVariants.filter((variant, index) => manifestVariants.indexOf(variant) !== index)
const missing = requestVariants.filter((variant) => !manifestVariants.includes(variant))
const stale = manifestVariants.filter((variant) => !requestVariants.includes(variant))
const allowed = new Set(["agent_terminal_supported", "presentation_only", "kernel_internal", "hosted_service_only"])
const invalid = entries.filter((entry) => !entry || typeof entry.variant !== "string" || !allowed.has(entry.classification) || typeof entry.reason !== "string" || !entry.reason.trim())
const supportedWithoutContract = entries
  .filter((entry) => entry.classification === "agent_terminal_supported")
  .filter((entry) => !contracts.contracts?.[entry.variant]?.input_schema)
const supportedVariants = new Set(entries.filter((entry) => entry.classification === "agent_terminal_supported").map((entry) => entry.variant))
const contractVariants = Object.keys(contracts.contracts ?? {})
const staleContracts = contractVariants.filter((variant) => !supportedVariants.has(variant))
const contractShapeErrors = entries
  .filter((entry) => entry.classification === "agent_terminal_supported")
  .flatMap((entry) => {
    const contract = contracts.contracts?.[entry.variant]
    const schema = contract?.input_schema
    const properties = schema?.type === "object" && schema.properties && typeof schema.properties === "object" ? schema.properties : {}
    const required = schema?.type === "object" && Array.isArray(schema.required) ? schema.required : []
    const errors = []
    if (!contract || !schema || !["object", "null"].includes(schema.type) || (schema.type === "object" && schema.additionalProperties !== false)) errors.push("schema must be a closed object or null")
    if (schema?.type === "object") {
      for (const field of required) if (typeof field !== "string" || !Object.prototype.hasOwnProperty.call(properties, field)) errors.push(`required field ${String(field)} is missing from properties`)
      for (const target of contract?.required_targets ?? []) if (typeof target !== "string" || !Object.prototype.hasOwnProperty.call(properties, target)) errors.push(`required target ${String(target)} is missing from properties`)
    }
    return errors.length ? [{ variant: entry.variant, errors }] : []
  })

if (duplicateVariants.length || missing.length || stale.length || invalid.length || supportedWithoutContract.length || staleContracts.length || contractShapeErrors.length) {
  console.error(JSON.stringify({ duplicateVariants, missing, stale, invalid, supportedWithoutContract, staleContracts, contractShapeErrors }, null, 2))
  process.exit(1)
}

const revision = `sha256:${createHash("sha256").update(JSON.stringify(entries)).digest("hex")}`
console.log(JSON.stringify({ ok: true, source: manifest.source, requests: entries.length, revision }))
