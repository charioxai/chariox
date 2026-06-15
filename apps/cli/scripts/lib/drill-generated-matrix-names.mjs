export const DRILL_GENERATED_MATRIX_NAMES = Object.freeze([
  "cloud-slice-runtime-matrix",
  "native-provider-tui-matrix",
  "remote-agent-runtime-matrix",
  "remote-home-extension-matrix",
  "slice-runtime-matrix",
  "workspace-live-sync-matrix",
])

export const DRILL_GENERATED_MATRIX_NAMES_BY_REPO = Object.freeze({
  cloud: Object.freeze(["cloud-slice-runtime-matrix"]),
  oss: Object.freeze([
    "native-provider-tui-matrix",
    "remote-agent-runtime-matrix",
    "remote-home-extension-matrix",
    "slice-runtime-matrix",
    "workspace-live-sync-matrix",
  ]),
})

export function isKnownDrillGeneratedMatrixName(matrixName) {
  return DRILL_GENERATED_MATRIX_NAMES.includes(matrixName)
}
