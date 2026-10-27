import { access, chmod, copyFile, mkdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

export async function prepareWorkspaceLiveSyncDaemonEnvironment({
  rootDir,
  daemonId,
  providers,
  sourceEnv = process.env,
  sourceHome = os.homedir(),
}) {
  const credentialRoot = path.join(rootDir, `${daemonId}-provider-profile`)
  const codexHome = path.join(credentialRoot, 'codex')
  const xdgDataHome = path.join(credentialRoot, 'xdg-data')
  const opencodeDataHome = path.join(xdgDataHome, 'opencode')
  const opencodeConfigDir = path.join(credentialRoot, 'opencode-config')
  const selected = new Set(providers)

  await mkdir(path.join(rootDir, `${daemonId}-home`), { recursive: true })
  await mkdir(codexHome, { recursive: true, mode: 0o700 })
  await mkdir(opencodeDataHome, { recursive: true, mode: 0o700 })
  await mkdir(opencodeConfigDir, { recursive: true, mode: 0o700 })

  if (selected.has('codex')) {
    const sourceCodexHome = sourceEnv.CODEX_HOME?.trim() || path.join(sourceHome, '.codex')
    await copyCredentialIfPresent(
      path.join(sourceCodexHome, 'auth.json'),
      path.join(codexHome, 'auth.json'),
    )
  }
  if (selected.has('opencode')) {
    const sourceOpenCodeDataHome = sourceEnv.OPENCODE_DATA_HOME?.trim()
      || (sourceEnv.XDG_DATA_HOME?.trim()
        ? path.join(sourceEnv.XDG_DATA_HOME, 'opencode')
        : path.join(sourceHome, '.local', 'share', 'opencode'))
    await copyCredentialIfPresent(
      path.join(sourceOpenCodeDataHome, 'auth.json'),
      path.join(opencodeDataHome, 'auth.json'),
    )
  }

  return {
    credentialRoot,
    env: {
      HOME: path.join(rootDir, `${daemonId}-home`),
      XDG_CONFIG_HOME: path.join(rootDir, `${daemonId}-xdg-config`),
      XDG_STATE_HOME: path.join(rootDir, `${daemonId}-xdg-state`),
      XDG_DATA_HOME: xdgDataHome,
      XDG_CACHE_HOME: path.join(rootDir, `${daemonId}-xdg-cache`),
      CODEX_HOME: codexHome,
      OPENCODE_DATA_HOME: opencodeDataHome,
      OPENCODE_CONFIG_DIR: opencodeConfigDir,
    },
  }
}

export async function removeWorkspaceLiveSyncProviderProfile(profile) {
  if (!profile?.credentialRoot) return
  await rm(profile.credentialRoot, { recursive: true, force: true })
}

async function copyCredentialIfPresent(source, destination) {
  try {
    await access(source)
  } catch {
    return false
  }
  await mkdir(path.dirname(destination), { recursive: true, mode: 0o700 })
  await copyFile(source, destination)
  await chmod(destination, 0o600)
  return true
}
