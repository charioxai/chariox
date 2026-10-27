import {
  addPendingPromptAttachments,
  filterPendingPromptAttachmentsForText,
  removePendingPromptAttachmentToken,
  type PendingPromptAttachment,
  type StoredPromptAttachment,
} from "./prompt-attachment-state.js"
import { resolvePromptAttachmentEdit } from "./prompt-attachments.js"

type PromptAttachmentSelection = {
  start: number
  end: number
}

export type PromptAttachmentInput = {
  plainText: string
  cursorOffset: number
  getSelection: () => PromptAttachmentSelection | null | undefined
}

export type PromptAttachmentControllerDeps = {
  getPromptInput: () => PromptAttachmentInput | null
  getPromptText: () => string
  setPromptText: (value: string) => void
  pendingAttachments: () => PendingPromptAttachment[]
  setPendingAttachments: (attachments: PendingPromptAttachment[]) => void
  updatePendingAttachments: (
    updater: (attachments: PendingPromptAttachment[]) => PendingPromptAttachment[],
  ) => void
  refreshHighlights: () => void
  updateSessionChrome: () => void
  requestRender: () => void
}

export function createPromptAttachmentController(deps: PromptAttachmentControllerDeps) {
  const requestAttachmentRender = () => {
    deps.updateSessionChrome()
    deps.requestRender()
  }

  const clear = () => {
    deps.setPendingAttachments([])
    deps.refreshHighlights()
    requestAttachmentRender()
  }

  const syncFromText = (value: string) => {
    deps.setPendingAttachments(filterPendingPromptAttachmentsForText(deps.pendingAttachments(), value))
    deps.refreshHighlights()
  }

  const insertTokens = (tokens: string[], at: number) => {
    if (tokens.length === 0) {
      return
    }
    const current = deps.getPromptText()
    const prefix = current.slice(0, at)
    const suffix = current.slice(at)
    const before = prefix && !/\s$/.test(prefix) ? " " : ""
    const after = suffix && !/^\s/.test(suffix) ? " " : ""
    const content = tokens.join(" ")
    const next = `${prefix}${before}${content}${after}${suffix}`
    deps.setPromptText(next)
    const input = deps.getPromptInput()
    if (input) {
      input.cursorOffset = prefix.length + before.length + content.length
    }
  }

  const removeToken = (token: string) => {
    const result = removePendingPromptAttachmentToken({
      attachments: deps.pendingAttachments(),
      text: deps.getPromptText(),
      token,
    })
    deps.setPendingAttachments(result.nextAttachments)
    if (!result.tokenFound) {
      deps.refreshHighlights()
      return false
    }
    deps.setPromptText(result.nextText)
    const input = deps.getPromptInput()
    if (input) {
      input.cursorOffset = result.cursorOffset ?? input.cursorOffset
    }
    return true
  }

  const removeLast = () => {
    const last = deps.pendingAttachments().at(-1)
    if (!last) {
      return false
    }
    removeToken(last.token)
    requestAttachmentRender()
    return true
  }

  const addStoredFiles = (files: readonly StoredPromptAttachment[], at: number) => {
    const result = addPendingPromptAttachments(deps.pendingAttachments(), files)
    if (result.addedAttachments.length === 0) {
      return false
    }
    deps.setPendingAttachments(result.nextAttachments)
    insertTokens(result.addedAttachments.map((file) => file.token), at)
    deps.refreshHighlights()
    requestAttachmentRender()
    return true
  }

  const removeForEdit = (action: "backspace" | "delete") => {
    const input = deps.getPromptInput()
    if (!input) {
      return false
    }
    const text = input.plainText
    const selection = input.getSelection()
    const cursor = input.cursorOffset
    const edit = resolvePromptAttachmentEdit(
      text,
      deps.pendingAttachments().map((file) => file.token),
      action,
      cursor,
      selection,
    )
    if (!edit) {
      return false
    }
    if (edit.kind === "noop") {
      return true
    }
    if (edit.kind === "delete-text") {
      const hasSelection = selection && selection.start !== selection.end
      deps.setPromptText(`${text.slice(0, edit.start)}${text.slice(edit.end)}`)
      input.cursorOffset = hasSelection ? Math.min(selection.start, selection.end) : cursor
      requestAttachmentRender()
      return true
    }
    deps.updatePendingAttachments((files) => files.filter((file) => !edit.tokens.includes(file.token)))
    deps.setPromptText(`${text.slice(0, edit.start)}${text.slice(edit.end)}`)
    input.cursorOffset = edit.start
    requestAttachmentRender()
    return true
  }

  return {
    addStoredFiles,
    clear,
    insertTokens,
    removeForEdit,
    removeLast,
    removeToken,
    syncFromText,
  }
}
