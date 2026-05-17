import {
  describeCliDialogFocusTarget,
  type CliDialogFocusSnapshot,
  type CliDialogFocusTarget,
} from "./cli-dialog-focus-controller.js"

type FocusAwareRenderer = {
  currentFocusedRenderable?: CliDialogFocusTarget | null
}

export function createCliRendererFocusController(
  renderer: unknown,
): {
  current: () => CliDialogFocusTarget | null
  describe: (focus: unknown) => CliDialogFocusSnapshot | null
} {
  const current = (): CliDialogFocusTarget | null => {
    return (renderer as FocusAwareRenderer).currentFocusedRenderable ?? null
  }

  const describe = (focus: unknown): CliDialogFocusSnapshot | null => {
    return describeCliDialogFocusTarget(focus as CliDialogFocusTarget | null | undefined)
  }

  return {
    current,
    describe,
  }
}
