import { readFile } from "node:fs/promises"
import { isAbsolute, join, relative, resolve, sep } from "node:path"

import { createWorkflowEventBindingRequest } from "@chariox/kernel-client/ipc-requests"

import type { KernelLookupClient, WorkflowPublicationPackage } from "./publication-types.js"

interface MaterializedEventBinding {
  readonly source_binding_id: string
  readonly generator_id: string
  readonly generator_version: string
  readonly manifest_digest: string
  readonly event_type: string
  readonly event_type_version: number
  readonly filter: unknown
  readonly requested_scope: string
  readonly endpoint_id: string
  readonly queue_ref: string | null
  readonly reply_mode: "disabled" | "thread" | "channel"
  readonly action_ids: readonly string[]
  readonly activation: {
    readonly connection_id: string
    readonly environment_id: string
    readonly mode: "authorized"
  }
}

export async function activatePublicationEventBindings(input: {
  readonly client: KernelLookupClient
  readonly packageRoot: string
  readonly publicationPackage: WorkflowPublicationPackage
  readonly runtimeSessionId: string
}): Promise<void> {
  const configuredPath = input.publicationPackage.event_bindings_path
  if (configuredPath === undefined) return
  if (configuredPath !== "event-bindings.local.json") {
    throw new Error("hosted publication event_bindings_path must be event-bindings.local.json")
  }
  const path = containedPackagePath(input.packageRoot, configuredPath)
  const document = parseEventBindings(JSON.parse(await readFile(path, "utf8")), input.publicationPackage.publication_id)
  for (const binding of document.bindings) {
    const response = await input.client.send(createWorkflowEventBindingRequest(
      input.runtimeSessionId,
      input.publicationPackage.publication_id,
      {
        generatorId: binding.generator_id,
        generatorVersion: binding.generator_version,
        manifestDigest: binding.manifest_digest,
        connectionId: binding.activation.connection_id,
        connectionScope: binding.requested_scope,
        eventType: binding.event_type,
        eventTypeVersion: binding.event_type_version,
        filter: binding.filter,
        environmentId: binding.activation.environment_id,
        queueRef: binding.queue_ref,
        replyMode: binding.reply_mode,
        actionIds: binding.action_ids,
      },
    ))
    if (!objectRecord(response)?.WorkflowEventBindingCreated) {
      throw new Error(`kernel did not activate publication event binding ${binding.source_binding_id}`)
    }
  }
}

function parseEventBindings(value: unknown, publicationId: string): {
  readonly bindings: readonly MaterializedEventBinding[]
} {
  const record = exactObject(value, [
    "schema_version",
    "publication_id",
    "destination_environment_id",
    "secrets_included",
    "bindings",
  ], "publication event bindings")
  if (
    record.schema_version !== 1
    || record.publication_id !== publicationId
    || record.secrets_included !== false
    || !Array.isArray(record.bindings)
    || record.bindings.length === 0
    || record.bindings.length > 256
  ) {
    throw new Error("publication event bindings are invalid")
  }
  const destinationEnvironmentId = requiredString(
    record.destination_environment_id,
    "publication event destination_environment_id",
  )
  const seen = new Set<string>()
  return { bindings: record.bindings.map((candidate): MaterializedEventBinding => {
    const binding = exactObject(candidate, [
      "source_binding_id",
      "generator_id",
      "generator_version",
      "manifest_digest",
      "event_type",
      "event_type_version",
      "filter",
      "requested_scope",
      "endpoint_id",
      "queue_ref",
      "reply_mode",
      "action_ids",
      "source_environment_id",
      "source_revision",
      "activation",
    ], "publication event binding")
    const sourceBindingId = requiredString(binding.source_binding_id, "publication event binding source_binding_id")
    if (seen.has(sourceBindingId)) throw new Error(`publication event binding ${sourceBindingId} is repeated`)
    seen.add(sourceBindingId)
    const activation = exactObject(binding.activation, ["connection_id", "environment_id", "mode"], "publication event activation")
    if (activation.mode !== "authorized" || activation.environment_id !== destinationEnvironmentId) {
      throw new Error(`publication event binding ${sourceBindingId} has an unauthorized destination`)
    }
    const eventTypeVersion = positiveInteger(binding.event_type_version, `publication event binding ${sourceBindingId} event_type_version`)
    if (!Number.isSafeInteger(binding.source_revision) || (binding.source_revision as number) < 1) {
      throw new Error(`publication event binding ${sourceBindingId} source_revision is invalid`)
    }
    const replyMode = binding.reply_mode
    if (replyMode !== "disabled" && replyMode !== "thread" && replyMode !== "channel") {
      throw new Error(`publication event binding ${sourceBindingId} reply_mode is invalid`)
    }
    if (!Array.isArray(binding.action_ids) || binding.action_ids.some((item) => typeof item !== "string" || !item.trim())) {
      throw new Error(`publication event binding ${sourceBindingId} action_ids are invalid`)
    }
    return {
      source_binding_id: sourceBindingId,
      generator_id: requiredString(binding.generator_id, `publication event binding ${sourceBindingId} generator_id`),
      generator_version: requiredString(binding.generator_version, `publication event binding ${sourceBindingId} generator_version`),
      manifest_digest: requiredString(binding.manifest_digest, `publication event binding ${sourceBindingId} manifest_digest`),
      event_type: requiredString(binding.event_type, `publication event binding ${sourceBindingId} event_type`),
      event_type_version: eventTypeVersion,
      filter: binding.filter ?? null,
      requested_scope: requiredString(binding.requested_scope, `publication event binding ${sourceBindingId} requested_scope`),
      endpoint_id: requiredString(binding.endpoint_id, `publication event binding ${sourceBindingId} endpoint_id`),
      queue_ref: optionalString(binding.queue_ref, `publication event binding ${sourceBindingId} queue_ref`),
      reply_mode: replyMode,
      action_ids: [...binding.action_ids],
      activation: {
        connection_id: requiredString(activation.connection_id, `publication event binding ${sourceBindingId} connection_id`),
        environment_id: destinationEnvironmentId,
        mode: "authorized",
      },
    }
  }) }
}

function containedPackagePath(root: string, configuredPath: string): string {
  if (!configuredPath.trim() || isAbsolute(configuredPath)) {
    throw new Error("publication event_bindings_path is invalid")
  }
  const target = resolve(root, configuredPath)
  const pathname = relative(resolve(root), target)
  if (!pathname || pathname === ".." || pathname.startsWith(`..${sep}`) || isAbsolute(pathname)) {
    throw new Error("publication event_bindings_path escapes its package")
  }
  return join(resolve(root), pathname)
}

function exactObject(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const record = objectRecord(value)
  if (!record || Object.keys(record).length !== keys.length || keys.some((key) => !(key in record))) {
    throw new Error(`${label} fields are invalid`)
  }
  return record
}

function positiveInteger(value: unknown, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 1) throw new Error(`${label} is invalid`)
  return value as number
}

function requiredString(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim() || value.length > 2_000) throw new Error(`${label} is required`)
  return value
}

function optionalString(value: unknown, label: string): string | null {
  if (value === null || value === undefined) return null
  return requiredString(value, label)
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
