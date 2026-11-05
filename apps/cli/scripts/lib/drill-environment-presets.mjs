const REMOTE_KERNEL_PASSTHROUGH_FLAGS = new Set([
  "--remote-kernel-host",
  "--remote-kernel-key",
  "--remote-kernel-repo",
])

const REMOTE_KERNEL_METADATA_KEYS = {
  "--remote-kernel-host": "hasRemoteKernelHostOverride",
  "--remote-kernel-key": "hasRemoteKernelSshIdentityOverride",
  "--remote-kernel-repo": "hasRemoteKernelRepoOverride",
}

export const DRILL_DEPLOYMENT_PRESETS = Object.freeze([
  "hosted-cloud",
  "local",
  "remote-kernel",
  "same-host-remote",
  "self-hosted-relay",
])

const DEPLOYMENT_PRESET_FLAGS = {
  "remote-kernel": "includesRemoteKernel",
  "hosted-cloud": "includesHostedCloud",
  "local": "includesLocal",
  "same-host-remote": "includesSameHostRemote",
  "self-hosted-relay": "includesSelfHostedRelay",
}

export function parseRemoteKernelPassthroughArg(argv, index) {
  const arg = argv[index]
  if (!arg) return null
  const equalsIndex = arg.indexOf("=")
  const flag = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex)
  if (!REMOTE_KERNEL_PASSTHROUGH_FLAGS.has(flag)) return null

  if (equalsIndex !== -1) {
    const value = arg.slice(equalsIndex + 1)
    if (!value) throw new Error(`${flag} requires a value`)
    return { args: [flag, value], nextIndex: index }
  }

  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return { args: [flag, value], nextIndex: index + 1 }
}

export function appendRemoteKernelPassthrough(baseArgs, scenario, passthrough) {
  const requires = Array.isArray(scenario.requires) ? scenario.requires : []
  return requires.includes("remote-kernel") ? [...baseArgs, ...passthrough] : baseArgs
}

export function remoteKernelPassthroughMetadata(passthrough) {
  const metadata = {
    hasRemoteKernelHostOverride: false,
    hasRemoteKernelSshIdentityOverride: false,
    hasRemoteKernelRepoOverride: false,
  }
  for (let index = 0; index < passthrough.length; index += 2) {
    const key = REMOTE_KERNEL_METADATA_KEYS[passthrough[index]]
    if (key) metadata[key] = true
  }
  return metadata
}

export function drillDeploymentPresetMetadata(presets, { remoteKernelPassthrough = [] } = {}) {
  const normalized = [...new Set((presets ?? []).map((preset) => String(preset).trim()).filter(Boolean))].sort()
  validateDrillDeploymentPresets(normalized, "drill deployment presets")
  const metadata = {
    deploymentPresetCount: normalized.length,
    deploymentPresets: normalized.join(","),
    includesRemoteKernel: false,
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
    ...remoteKernelPassthroughMetadata(remoteKernelPassthrough),
  }
}

export function isKnownDrillDeploymentPreset(preset) {
  return DRILL_DEPLOYMENT_PRESETS.includes(preset)
}

export function validateDrillDeploymentPreset(preset, source, { message } = {}) {
  if (!isKnownDrillDeploymentPreset(preset)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(preset) : message)
    }
    throw new Error(`${source} has unknown deployment preset ${JSON.stringify(preset)}`)
  }
}

export function validateDrillDeploymentPresets(presets, source) {
  if (!Array.isArray(presets) || !presets.every((preset) => typeof preset === "string" && preset.length > 0)) {
    throw new Error(`${source} has invalid deployment presets`)
  }
  for (const [index, preset] of presets.entries()) {
    validateDrillDeploymentPreset(preset, `${source}[${index}]`)
  }
}
