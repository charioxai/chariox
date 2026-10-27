export const DRILL_GENERATED_MATRIX_NAMES = Object.freeze([
  "browser-terminal-resilience-matrix",
  "cloud-slice-runtime-matrix",
  "native-provider-tui-matrix",
  "remote-agent-runtime-matrix",
  "remote-home-extension-matrix",
  "runtime-resilience-chaos-matrix",
  "slice-runtime-matrix",
  "workspace-live-sync-matrix",
])

export const DRILL_GENERATED_MATRIX_NAMES_BY_REPO = Object.freeze({
  cloud: Object.freeze(["browser-terminal-resilience-matrix", "cloud-slice-runtime-matrix"]),
  oss: Object.freeze([
    "native-provider-tui-matrix",
    "remote-agent-runtime-matrix",
    "runtime-resilience-chaos-matrix",
    "remote-home-extension-matrix",
    "slice-runtime-matrix",
    "workspace-live-sync-matrix",
  ]),
})

export const DRILL_GENERATED_MATRIX_NAMES_SCHEMA = "chariox.drill.generated_matrix_names.v1"

export function drillGeneratedMatrixNamesManifest() {
  return {
    schema: DRILL_GENERATED_MATRIX_NAMES_SCHEMA,
    matrices: DRILL_GENERATED_MATRIX_NAMES.map((name) => ({
      name,
      repo: repoForGeneratedMatrixName(name),
    })),
  }
}

export function isKnownDrillGeneratedMatrixName(matrixName) {
  return DRILL_GENERATED_MATRIX_NAMES.includes(matrixName)
}

export function drillGeneratedMatrixRepoForName(matrixName) {
  return repoForGeneratedMatrixName(matrixName)
}

export function validateDrillGeneratedMatrixNamesManifest(manifest, source = "generated matrix names manifest") {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_GENERATED_MATRIX_NAMES_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.matrices)) {
    throw new Error(`${source} has invalid matrices`)
  }
  const names = manifest.matrices.map((matrix) => matrix?.name).sort()
  if (JSON.stringify(names) !== JSON.stringify(DRILL_GENERATED_MATRIX_NAMES)) {
    throw new Error(`${source} matrices do not match generated matrix name registry`)
  }
  for (const [index, matrix] of manifest.matrices.entries()) {
    const matrixSource = `${source}.matrices[${index}]`
    if (!matrix || typeof matrix !== "object" || Array.isArray(matrix)) {
      throw new Error(`${matrixSource} is not an object`)
    }
    if (!isKnownDrillGeneratedMatrixName(matrix.name)) {
      throw new Error(`${matrixSource} has unknown name ${JSON.stringify(matrix.name)}`)
    }
    if (matrix.repo !== repoForGeneratedMatrixName(matrix.name)) {
      throw new Error(`${matrixSource} has invalid repo ${JSON.stringify(matrix.repo)}`)
    }
  }
}

function repoForGeneratedMatrixName(matrixName) {
  for (const [repo, matrixNames] of Object.entries(DRILL_GENERATED_MATRIX_NAMES_BY_REPO)) {
    if (matrixNames.includes(matrixName)) return repo
  }
  return null
}
