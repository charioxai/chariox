// One authoring list; Rust includes the JSON and clients use this checked projection.
export function runtimeCardTypesSource(entries) {
  return `// Generated from docs/runtime-card-types.json; edit the authoritative list, not this file.\nexport const runtimeCardTypes: Readonly<Record<string, string>> = Object.freeze(${JSON.stringify(Object.fromEntries(entries.map(entry => [entry.type, entry.class])), null, 2)})\n\nexport function runtimeCardClass(type: string): string | undefined {\n  return Object.hasOwn(runtimeCardTypes, type) ? runtimeCardTypes[type] : undefined\n}\n`
}
