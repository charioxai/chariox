const HETZNER_PASSTHROUGH_FLAGS = new Set([
  "--hetzner-host",
  "--hetzner-key",
  "--hetzner-repo",
])

const HETZNER_METADATA_KEYS = {
  "--hetzner-host": "hasHetznerHostOverride",
  "--hetzner-key": "hasHetznerSshIdentityOverride",
  "--hetzner-repo": "hasHetznerRepoOverride",
}

export function parseHetznerPassthroughArg(argv, index) {
  const arg = argv[index]
  if (!arg) return null
  const equalsIndex = arg.indexOf("=")
  const flag = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex)
  if (!HETZNER_PASSTHROUGH_FLAGS.has(flag)) return null

  if (equalsIndex !== -1) {
    const value = arg.slice(equalsIndex + 1)
    if (!value) throw new Error(`${flag} requires a value`)
    return { args: [flag, value], nextIndex: index }
  }

  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return { args: [flag, value], nextIndex: index + 1 }
}

export function appendHetznerPassthrough(baseArgs, scenario, passthrough) {
  const requires = Array.isArray(scenario.requires) ? scenario.requires : []
  return requires.includes("hetzner") ? [...baseArgs, ...passthrough] : baseArgs
}

export function hetznerPassthroughMetadata(passthrough) {
  const metadata = {
    hasHetznerHostOverride: false,
    hasHetznerSshIdentityOverride: false,
    hasHetznerRepoOverride: false,
  }
  for (let index = 0; index < passthrough.length; index += 2) {
    const key = HETZNER_METADATA_KEYS[passthrough[index]]
    if (key) metadata[key] = true
  }
  return metadata
}
