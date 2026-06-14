import path from "node:path"

export function validationGateEvidenceSourceMetadata(report, {
  ossRoot,
  cloudRoot,
} = {}) {
  const roots = [
    ["oss", ossRoot],
    ["cloud", cloudRoot],
  ].filter(([, root]) => typeof root === "string" && root.length > 0)
    .map(([name, root]) => [name, path.resolve(root)])

  const matrixRepos = sourceReposForPaths(matrixEvidencePaths(report), roots)
  const artifactRepos = sourceReposForPaths(artifactEvidencePaths(report), roots)
  const failureRepos = sourceReposForPaths(failureEvidencePaths(report), roots)
  const evidenceRepos = sortedUnique([
    ...matrixRepos,
    ...artifactRepos,
    ...failureRepos,
  ])

  return {
    ...(evidenceRepos.length > 0 ? { evidenceRepos: evidenceRepos.join(",") } : {}),
    ...(matrixRepos.length > 0 ? { matrixEvidenceRepos: matrixRepos.join(",") } : {}),
    ...(artifactRepos.length > 0 ? { artifactEvidenceRepos: artifactRepos.join(",") } : {}),
    ...(failureRepos.length > 0 ? { failureEvidenceRepos: failureRepos.join(",") } : {}),
  }
}

function matrixEvidencePaths(report) {
  return (report?.checks?.matrices?.aggregate?.reports ?? [])
    .map((entry) => entry?.source)
    .filter(nonEmptyString)
}

function artifactEvidencePaths(report) {
  return (report?.checks?.artifacts?.aggregate?.indexes ?? [])
    .flatMap((entry) => [entry?.source, entry?.rootDir])
    .filter(nonEmptyString)
}

function failureEvidencePaths(report) {
  return (report?.checks?.failures?.aggregate?.failures ?? [])
    .flatMap((entry) => [entry?.source, entry?.rootDir])
    .filter(nonEmptyString)
}

function sourceReposForPaths(paths, roots) {
  return sortedUnique(paths.map((sourcePath) => sourceRepoForPath(sourcePath, roots)).filter(nonEmptyString))
}

function sourceRepoForPath(sourcePath, roots) {
  const resolved = path.resolve(sourcePath)
  for (const [name, root] of roots) {
    if (resolved === root || resolved.startsWith(`${root}${path.sep}`)) {
      return name
    }
  }
  return "external"
}

function sortedUnique(values) {
  return [...new Set(values)].sort()
}

function nonEmptyString(value) {
  return typeof value === "string" && value.length > 0
}
