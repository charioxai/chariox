import { buildCommandCenterItems } from "./command-center.js"
import {
  commandCenterCompletionText,
  commandCenterExecutionCommand,
  nextCommandCenterIndex,
  shouldBypassCommandCenterSubmitSelection,
  shouldSubmitExactCommandCenterMatch,
} from "./command-center-selection.js"
import type { CommandCenterItem } from "./command-center-types.js"
import type { CommandNode } from "./command-center-tree-projection.js"
import type {
  CommandCenterWorkflowRegistryEntry,
} from "./command-center-context.js"
import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

export type CommandCenterKeyEvent = {
  name: string
  ctrl?: boolean
  eventType?: string
  preventDefault?: () => void
  stopPropagation?: () => void
}

export type CommandCenterRenderState = {
  open: boolean
  query: string
  items: readonly CommandCenterItem[]
  selectedIndex: number
}

type CommandCenterControllerOptions<TBox = unknown> = {
  getCommandTree: () => readonly CommandNode[]
  getProviderCatalog: () => ProviderCatalog
  getProviderCommandCatalogs: () => ProviderCommandCatalogs
  getCurrentProvider: () => BackendProviderId
  getFocusedProvider: () => BackendProviderId | null
  getCurrentModel: () => string
  getCurrentVariant: () => string
  getWorkflowRegistryEntries?: () => readonly CommandCenterWorkflowRegistryEntry[]
  refreshWorkflowRegistryEntries?: (input: string) => void
  getPromptText: () => string
  replacePromptText: (value: string) => void
  executeCommand: (value: string) => Promise<void>
  onCommandError: (error: unknown) => void
  render: (state: CommandCenterRenderState, box: TBox | undefined) => void
}

export type CommandCenterController<TBox = unknown> = {
  query(): string
  items(): readonly CommandCenterItem[]
  selectedIndex(): number
  selectedItem(): CommandCenterItem | null
  open(): boolean
  assignBox(value: TBox | undefined): void
  sync(value?: string): void
  clear(): void
  moveSelection(delta: number): void
  handleKey(event: CommandCenterKeyEvent): boolean
  selectFromSubmit(): boolean
  render(): void
}

export function createCommandCenterController<TBox = unknown>(
  options: CommandCenterControllerOptions<TBox>,
): CommandCenterController<TBox> {
  let query = ""
  let items: CommandCenterItem[] = []
  let selectedIndex = 0
  let box: TBox | undefined

  const open = () => items.length > 0 && query.startsWith("/")

  const render = () => {
    options.render({
      open: open(),
      query,
      items,
      selectedIndex,
    }, box)
  }

  const replacePromptText = (value: string) => {
    options.replacePromptText(value)
  }

  const sync = (value = options.getPromptText()) => {
    const previousValue = query
    query = value
    options.refreshWorkflowRegistryEntries?.(value)
    items = buildCommandCenterItems(value, {
      providerCatalog: options.getProviderCatalog(),
      commandTree: options.getCommandTree(),
      providerCommandCatalogs: options.getProviderCommandCatalogs(),
      currentProvider: options.getCurrentProvider(),
      focusedProvider: options.getFocusedProvider(),
      currentModel: options.getCurrentModel(),
      currentVariant: options.getCurrentVariant(),
      ...(options.getWorkflowRegistryEntries ? { workflowRegistryEntries: options.getWorkflowRegistryEntries() } : {}),
    })
    selectedIndex = nextCommandCenterIndex(selectedIndex, items, value, previousValue)
    render()
  }

  const clear = () => {
    query = ""
    items = []
    selectedIndex = 0
    render()
  }

  const selectedItem = () => items[selectedIndex] ?? items[0] ?? null

  const selectItem = async (item: CommandCenterItem) => {
    const command = commandCenterExecutionCommand(item)
    if (command === null) {
      const completionText = commandCenterCompletionText(item)
      replacePromptText(completionText)
      sync(completionText)
      return
    }

    const clearsBeforeExecution = item.kind === "command" || item.kind === "group"
    try {
      if (clearsBeforeExecution) {
        clear()
        replacePromptText("")
      }
      await options.executeCommand(command)
    } catch (error) {
      options.onCommandError(error)
    } finally {
      if (!clearsBeforeExecution) {
        replacePromptText("")
        sync("")
      }
    }
  }

  const completeItem = (item: CommandCenterItem) => {
    const completionText = commandCenterCompletionText(item)
    replacePromptText(completionText)
    sync(completionText)
  }

  const moveSelection = (delta: number) => {
    if (items.length === 0) {
      return
    }
    selectedIndex = (selectedIndex + delta + items.length) % items.length
    render()
  }

  return {
    query() {
      return query
    },
    items() {
      return items
    },
    selectedIndex() {
      return selectedIndex
    },
    selectedItem,
    open,
    assignBox(value) {
      box = value
    },
    sync,
    clear,
    moveSelection,
    handleKey(event) {
      if (!open() || event.eventType === "release") {
        return false
      }
      if (event.name === "up" || (event.ctrl && event.name === "p")) {
        event.preventDefault?.()
        event.stopPropagation?.()
        moveSelection(-1)
        return true
      }
      if (event.name === "down" || (event.ctrl && event.name === "n")) {
        event.preventDefault?.()
        event.stopPropagation?.()
        moveSelection(1)
        return true
      }
      if (event.name === "escape") {
        event.preventDefault?.()
        event.stopPropagation?.()
        clear()
        return true
      }
      if (event.name === "return" || event.name === "enter") {
        const item = selectedItem()
        if (!item) {
          return false
        }
        event.preventDefault?.()
        event.stopPropagation?.()
        void selectItem(item)
        return true
      }
      if (event.name === "tab") {
        const item = selectedItem()
        if (!item) {
          return false
        }
        event.preventDefault?.()
        event.stopPropagation?.()
        completeItem(item)
        return true
      }
      return false
    },
    selectFromSubmit() {
      const item = selectedItem()
      if (!item) {
        return false
      }
      const currentPrompt = options.getPromptText()
      if (shouldBypassCommandCenterSubmitSelection(currentPrompt)) {
        return false
      }
      if (shouldSubmitExactCommandCenterMatch(item, currentPrompt)) {
        clear()
        sync("")
        return false
      }
      void selectItem(item)
      return true
    },
    render,
  }
}
