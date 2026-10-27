// MP-01/MP-04/MP-07/MP-11: signed Path-1 units must launch an ordinary provider.
// Admit only the service settings used by the reviewed Path-1 units. systemd
// keeps adding execution restrictions; a deny list cannot protect parity from
// a new or previously unlisted filesystem, process, network or device policy.
const supportedServiceDirectives = new Set([
  "Type", "User", "Group", "Environment", "ExecStartPre", "ExecStart",
  "ExecStartPost", "Delegate", "DelegateSubgroup", "Restart", "RestartSec",
  "RestartSteps", "RestartMaxDelaySec", "KillMode", "StateDirectory",
  "StateDirectoryMode",
])

export function parseUnitSections(source) {
  const sections = new Map()
  let section
  for (const rawLine of source.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith("#") || line.startsWith(";")) continue
    const header = /^\[([^\]]+)\]$/.exec(line)
    if (header) {
      section = header[1]
      if (!sections.has(section)) sections.set(section, [])
      continue
    }
    // systemd ignores whitespace around the first assignment '='. Normalize
    // before exact-count/prefix checks, including repeated section overrides.
    // Continuations and unparsed directives must never bypass the policy.
    const assignment = /^([A-Za-z][A-Za-z0-9]*)\s*=\s*(.*)$/.exec(line)
    if (!section || !assignment || line.endsWith("\\")) {
      throw new Error("service unit contains unsupported assignment syntax")
    }
    sections.get(section).push(`${assignment[1]}=${assignment[2]}`)
  }
  return sections
}

export function verifyPath1ServicePolicy(source, role) {
  const label = role === "worker" ? "Path-1 disposable-worker service" : "Path-1 managed bootstrap service"
  const fail = (message) => { throw new Error(`selected ${label} ${message}`) }
  const sections = parseUnitSections(source)
  const lines = sections.get("Service") ?? []
  const unit = sections.get("Unit") ?? []
  const all = [...sections.values()].flat()
  const exact = (prefix, required, entries = lines) => {
    const matches = all.filter((line) => line.startsWith(prefix))
    if (matches.length !== 1 || matches[0] !== required || !entries.includes(required)) {
      fail(`is missing or overrides ${required}`)
    }
  }
  const command = `ExecStart=/usr/local/bin/chariox-managed-bootstrap${role === "worker" ? " --disposable-worker" : ""}`
  if (all.filter((line) => line.startsWith("ExecStart=")).length !== 1 || !lines.includes(command)) {
    fail("has an incompatible ExecStart")
  }
  const path = "Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  if (all.filter((line) => line.startsWith("Environment=PATH=")).length !== 1 || !lines.includes(path)) {
    fail("has an incompatible bootstrap PATH")
  }
  for (const [name, value] of [
    ["CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "path1"],
    ["HOME", "/home/chariox"],
    ["CHARIOX_HOME", "/home/chariox/.chariox"],
    ["CHARIOX_SLICE_DOCKER_BROKER_SOCKET", "/var/lib/chariox-slice-share/.broker-private/control/control.sock"],
  ]) exact(`Environment=${name}=`, `Environment=${name}=${value}`)
  if (role === "home") exact("Environment=CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=", "Environment=CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=/etc/chariox/trusted-builder-public-key")
  exact("User=", "User=chariox")
  exact("Group=", "Group=chariox")
  exact("ExecStartPre=", "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service")
  const after = unit.filter((line) => line.startsWith("After="))
  if (after.length !== 1 || !after[0].slice("After=".length).trim().split(/\s+/).includes("chariox-rootless-docker.service")) {
    fail("must start after rootless Docker")
  }
  const forbidden = [
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "CHARIOX_MANAGED_PROVIDER_HOME",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
    "CHARIOX_SLICE_ROOT",
    "bwrap",
    "NoNewPrivileges=",
    "PrivateTmp=",
    "PrivateUsers=",
    "PrivateDevices=",
    "PrivateNetwork=",
    "PrivateMounts=",
    "ProtectSystem=",
    "ProtectHome=",
    "ProtectKernel",
    "ProtectControlGroups=",
    "RestrictNamespaces=",
    "RestrictAddressFamilies=",
    "RestrictSUIDSGID=",
    "ReadWritePaths=",
    "ReadOnlyPaths=",
    "InaccessiblePaths=",
    "TemporaryFileSystem=",
    "BindPaths=",
    "BindReadOnlyPaths=",
    "RootDirectory=",
    "RootImage=",
    "SystemCallFilter=",
    "IPAddressDeny=",
    "CapabilityBoundingSet=",
    "UMask=",
    "SupplementaryGroups=",
  ]
  for (const directive of forbidden) {
    if (all.some((line) => line.includes(directive))) fail(`contains ${directive}`)
  }
  const state = all.filter((line) => line.startsWith("StateDirectory="))
  const stateMode = all.filter((line) => line.startsWith("StateDirectoryMode="))
  if (role === "worker") {
    exact("StateDirectory=", "StateDirectory=chariox")
    exact("StateDirectoryMode=", "StateDirectoryMode=0700")
  } else if (state.length || stateMode.length) fail("contains StateDirectory=")
  // Only direct, unquoted one-assignment environment declarations are supported.
  // Otherwise a later quoted/multiple assignment could override a validated HOME.
  if (all.some((line) => /^(EnvironmentFile|PassEnvironment|UnsetEnvironment)=/.test(line)
    || line.startsWith("Environment=") && !/^Environment=[A-Z0-9_]+=[^\s"']+$/.test(line))) {
    fail("contains an unsupported environment override")
  }
  for (const line of lines) {
    const directive = line.slice(0, line.indexOf("="))
    if (!supportedServiceDirectives.has(directive)) {
      fail(`contains unsupported Service directive ${directive}`)
    }
  }
  return sections
}
