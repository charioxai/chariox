import type { MarkdownWorkspaceImage, MarkdownWorkspaceImageLoader } from "@chariox/transcript-view/markdown/markdown-images"
import type { TerminalTurnState } from "@chariox/transcript-view"

type Picture = { loading: Promise<MarkdownWorkspaceImage>; src?: string }

/** Share reads across redraws, until a completed turn may have replaced a workspace file. */
export class AgentTriggerImages {
  private readonly pictures = new Map<string, Picture>()
  private completed = ""
  constructor(private readonly read: (reference: string) => Promise<MarkdownWorkspaceImage>) {}

  refresh(turns: Iterable<TerminalTurnState>): void {
    const completed = [...turns].filter(turn => turn.kernelSettled || turn.completedAtMs != null)
      .map(turn => JSON.stringify([turn.id, turn.completedAtMs, turn.lastEntryAtMs, turn.kernelSettlementStatus])).sort().join("\n")
    if (completed === this.completed) return
    this.completed = completed
    this.clear()
  }

  readonly load: MarkdownWorkspaceImageLoader = (_element, reference) => {
    const known = this.pictures.get(reference)
    if (known) return known.loading
    const unavailable = { kind: "unavailable" as const, message: "image unavailable" }
    const picture: Picture = { loading: Promise.resolve(unavailable) }
    picture.loading = this.read(reference).then(image => {
      if (image.kind === "loaded") {
        // An older read can finish after completion invalidated it; it must not leak a blob or replace the new read.
        if (this.pictures.get(reference) !== picture) { URL.revokeObjectURL(image.src); return unavailable }
        picture.src = image.src
      } else if (this.pictures.get(reference) === picture) this.pictures.delete(reference)
      return image
    }, () => {
      if (this.pictures.get(reference) === picture) this.pictures.delete(reference)
      return unavailable
    })
    this.pictures.set(reference, picture)
    return picture.loading
  }

  clear(): void {
    for (const picture of this.pictures.values()) if (picture.src) URL.revokeObjectURL(picture.src)
    this.pictures.clear()
  }
}
