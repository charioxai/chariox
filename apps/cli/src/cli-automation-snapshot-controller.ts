import {
  buildCliAutomationSnapshot,
  type CliAutomationSnapshotDeps,
} from "./cli-automation-snapshot.js"

export type CliAutomationSnapshotControllerDeps = Omit<
  CliAutomationSnapshotDeps,
  "interactionChoiceSelection" | "interactionCustomReply" | "interactionCustomEditing"
> & {
  getInteractionChoiceSelection: (interactionId: string) => number | null | undefined
  getInteractionCustomReply: (interactionId: string) => string | null | undefined
  isInteractionCustomEditing: (interactionId: string) => boolean
}

export function createCliAutomationSnapshotController(
  deps: CliAutomationSnapshotControllerDeps,
) {
  return {
    snapshot: () => buildCliAutomationSnapshot({
      ...deps,
      interactionChoiceSelection: (interactionId) =>
        deps.getInteractionChoiceSelection(interactionId) ?? 0,
      interactionCustomReply: (interactionId) =>
        deps.getInteractionCustomReply(interactionId) ?? "",
      interactionCustomEditing: deps.isInteractionCustomEditing,
    }),
  }
}
