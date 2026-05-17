export function createInteractionChoiceStoreController() {
  const selectedChoices = new Map<string, number>()
  const customReplies = new Map<string, string>()
  const customEditing = new Set<string>()

  return {
    selectedChoiceIndex(interactionId: string) {
      return selectedChoices.get(interactionId) ?? 0
    },
    getSelectedIndex(interactionId: string) {
      return selectedChoices.get(interactionId)
    },
    setSelectedIndex(interactionId: string, index: number) {
      selectedChoices.set(interactionId, index)
    },
    customReply(interactionId: string) {
      return customReplies.get(interactionId) ?? ""
    },
    getStoredCustomReply(interactionId: string) {
      return customReplies.get(interactionId)
    },
    setCustomReply(interactionId: string, reply: string) {
      customReplies.set(interactionId, reply)
    },
    clearCustomReply(interactionId: string) {
      customReplies.delete(interactionId)
    },
    isCustomEditing(interactionId: string) {
      return customEditing.has(interactionId)
    },
    setCustomEditing(interactionId: string, editing: boolean) {
      if (editing) {
        customEditing.add(interactionId)
      } else {
        customEditing.delete(interactionId)
      }
    },
  }
}
