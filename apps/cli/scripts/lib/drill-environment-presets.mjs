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

export const DRILL_DEPLOYMENT_PRESETS = Object.freeze([
  "hetzner",
  "hosted-cloud",
  "local",
  "same-host-remote",
  "self-hosted-relay",
])

const DEPLOYMENT_PRESET_FLAGS = {
  "hetzner": "includesHetzner",
  "hosted-cloud": "includesHostedCloud",
  "local": "includesLocal",
  "same-host-remote": "includesSameHostRemote",
  "self-hosted-relay": "includesSelfHostedRelay",
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

export function drillDeploymentPresetMetadata(presets, { hetznerPassthrough = [] } = {}) {
  const normalized = [...new Set((presets ?? []).map((preset) => String(preset).trim()).filter(Boolean))].sort()
  validateDrillDeploymentPresets(normalized, "drill deployment presets")
  const metadata = {
    deploymentPresetCount: normalized.length,
    deploymentPresets: normalized.join(","),
    includesHetzner: false,
    includesHostedCloud: false,
    includesLocal: false,
    includesSameHostRemote: false,
    includesSelfHostedRelay: false,
  }
  for (const preset of normalized) {
    metadata[DEPLOYMENT_PRESET_FLAGS[preset]] = true
  }
  return {
    ...metadata,
    ...hetznerPassthroughMetadata(hetznerPassthrough),
  }
}

export function isKnownDrillDeploymentPreset(preset) {
  return DRILL_DEPLOYMENT_PRESETS.includes(preset)
}

export function validateDrillDeploymentPresets(presets, source) {
  if (!Array.isArray(presets) || !presets.every((preset) => typeof preset === "string" && preset.length > 0)) {
    throw new Error(`${source} has invalid deployment presets`)
  }
  for (const [index, preset] of presets.entries()) {
    if (!isKnownDrillDeploymentPreset(preset)) {
      throw new Error(`${source}[${index}] has unknown deployment preset ${JSON.stringify(preset)}`)
    }
  }
}
