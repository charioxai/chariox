import { readFileSync } from "node:fs"

const grammar = JSON.parse(readFileSync(new URL("./docker-image-reference.json", import.meta.url), "utf8"))
const reference = new RegExp(grammar.reference)

// Only typed image operands use this grammar. Docker resource mutation retains
// its separate slice/container/volume ownership checks.
export function normalizedImageReference(value) {
  if (typeof value !== "string") return null
  const match = reference.exec(value)
  if (!match) return null
  let [, domain, path, tag, digest] = match
  if (!domain || !(domain === "localhost" || /[.:\[]/.test(domain) || /[A-Z]/.test(domain))) {
    path = domain ? domain + "/" + path : path
    domain = "docker.io"
  }
  if (domain === "index.docker.io") domain = "docker.io"
  if (domain === "docker.io" && !path.includes("/")) path = "library/" + path
  if (path.length > grammar.maximumRepositoryPath) return null
  return domain + "/" + path + (tag ? ":" + tag : digest ? "" : ":latest") + (digest ? "@" + digest : "")
}

export function isDockerImageReference(value) { return normalizedImageReference(value) !== null }
export function isDefaultRuntimeBase(value) { return normalizedImageReference(value) === grammar.defaultBase }
