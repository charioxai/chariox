import { readFileSync } from "node:fs"

import type { CommandNode } from "./command-center-tree-projection.js"

const catalogFragmentUrls = [
  "../../kernel/src/runtime/terminal_command_catalog/catalog/core.json",
  "../../kernel/src/runtime/terminal_command_catalog/catalog/extensions.json",
  "../../kernel/src/runtime/terminal_command_catalog/catalog/workflow.json",
  "../../kernel/src/runtime/terminal_command_catalog/catalog/workspace.json",
  "../../kernel/src/runtime/terminal_command_catalog/catalog/provider.json",
]

export function loadCommandCenterTestCatalog(): CommandNode[] {
  return catalogFragmentUrls.flatMap((path) => (
    JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8")) as CommandNode[]
  ))
}
