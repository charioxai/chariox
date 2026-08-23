import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { promisify } from "node:util"
import { test } from "node:test"

const execFileAsync = promisify(execFile)
const repositoryRoot = resolve(import.meta.dirname, "..")
const entrypoint = await readFile(join(repositoryRoot, "docker/publication/entrypoint.sh"), "utf8")

test("publication entrypoint keeps builder actions outside credential and transport environments", () => {
  assert.match(entrypoint, /^umask 077$/m)
  assert.match(entrypoint, /runuser -u chariox-action -- env -i/)
  assert.match(entrypoint, /runuser -u chariox-gateway --preserve-environment/)
  assert.match(entrypoint, /launch_kernel_as_chariox "\$\{CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE:-\}"/)
  assert.match(entrypoint, /require_private_capability_file "\$kernel_local_auth_file" chariox-gateway/)
  assert.match(entrypoint, /CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE/)
  assert.match(entrypoint, /CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE/)
  assert.match(entrypoint, /read_bootstrap_capability_file/)
  assert.match(entrypoint, /O_RDONLY \| fs\.constants\.O_NOFOLLOW/)
  assert.match(entrypoint, /publication audit URL and URL file cannot both be configured/)
  assert.match(entrypoint, /chmod 700 "\$HOME"/)
  assert.match(entrypoint, /chmod 700 "\$CHARIOX_WORKSPACE_DIR"/)
  assert.match(entrypoint, /chmod -R go-rwx "\$CHARIOX_WORKSPACE_DIR"/)
  assert.match(entrypoint, /TMPDIR="\$CHARIOX_KERNEL_TMPDIR"/)
  assert.match(entrypoint, /TMPDIR="\$CHARIOX_GATEWAY_TMPDIR"/)
  assert.match(entrypoint, /TMPDIR="\$CHARIOX_ACTION_TMPDIR"/)
  assert.match(entrypoint, /O_EXCL/)
  assert.match(entrypoint, /O_NOFOLLOW/)
  assert.match(entrypoint, /renameSync\(temporary, destination\)/)
  assert.match(entrypoint, /CHARIOX_CAPABILITY_STAGING_DIR="\$CHARIOX_CAPABILITY_ROOT\/\.staging"/)
  assert.match(entrypoint, /first_unsafe_tree_path/)
  assert.match(entrypoint, /publication credential destination contains an unsafe path/)
  assert.match(entrypoint, /GATEWAY_PID="\$!"/)
  assert.match(entrypoint, /trap 'exit 143' TERM/)
  for (const name of [
    "CHARIOX_RELAY_URL",
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    "CHARIOX_RELAY_HEARTBEAT_MS",
    "CHARIOX_KERNEL_QUEUE_CAPACITY",
    "NODE_EXTRA_CA_CERTS",
    "HTTPS_PROXY",
    "NO_PROXY",
    "CHARIOX_PUBLICATION_CLOUD_API_URL",
    "CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID",
    "CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN",
  ]) assert.match(entrypoint, new RegExp(`\\b${name} \\\\`))
  assert.doesNotMatch(entrypoint, /chmod 755 "\$HOME"/)
  assert.doesNotMatch(entrypoint, /CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY/)
  assert.doesNotMatch(entrypoint, /printf '%s' "\$value" > "\$path"/)
})

test("publication entrypoint imports legacy and single-account credential bindings into runtime home", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-credentials-"))
  const home = join(root, "home")
  const bindings = join(root, "bindings")
  const codex = join(bindings, "000", "home", ".codex")
  const claude = join(bindings, "001", "home", ".claude")
  await mkdir(codex, { recursive: true })
  await mkdir(claude, { recursive: true })
  await writeFile(join(codex, "auth.json"), "codex-fixture")
  await writeFile(join(claude, ".credentials.json"), "claude-fixture")
  try {
    await execFileAsync("bash", [join(repositoryRoot, "docker/publication/entrypoint.sh"), "true"], {
      env: {
        ...process.env,
        HOME: home,
        CHARIOX_CONFIG_DIR: join(home, ".config", "chariox"),
        CHARIOX_DATA_DIR: join(home, ".local", "share", "chariox"),
        CHARIOX_RUNTIME_DIR: join(home, ".cache", "chariox", "runtime"),
        CHARIOX_SESSION_HISTORY_DIR: join(home, ".local", "share", "chariox", "sessions"),
        CHARIOX_CREDENTIAL_BINDINGS_ROOT: bindings,
        CHARIOX_PROVIDER_CREDENTIALS_DIR: join(root, "missing-legacy-profile"),
        CHARIOX_WORKSPACE_DIR: join(root, "workspace"),
      },
    })
    assert.equal(await readFile(join(home, ".codex", "auth.json"), "utf8"), "codex-fixture")
    assert.equal(await readFile(join(home, ".claude", ".credentials.json"), "utf8"), "claude-fixture")
    assert.equal((await stat(home)).mode & 0o777, 0o700)
    assert.equal((await stat(join(home, ".codex", "auth.json"))).mode & 0o077, 0)
    assert.equal((await stat(join(home, ".claude", ".credentials.json"))).mode & 0o077, 0)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication entrypoint imports only the designated default when multiple accounts share a provider", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-multi-account-"))
  const home = join(root, "home")
  const bindings = join(root, "bindings")
  for (const [index, profileId] of [["000", "profile-main"], ["001", "profile-secondary"]]) {
    const profile = join(bindings, index)
    await mkdir(join(profile, "home", ".codex"), { recursive: true })
    await writeFile(join(profile, "profile.json"), JSON.stringify({
      kind: "provider",
      provider: "codex",
      profileId,
    }))
    await writeFile(join(profile, "home", ".codex", "auth.json"), profileId)
  }
  const accounts = [
    { agent_id: "agent-1", provider: "codex", account_profile: "profile-main", label: "Main", home: `${bindings}/000/home` },
    { agent_id: "agent-2", provider: "codex", account_profile: "profile-secondary", label: "Secondary", home: `${bindings}/001/home` },
  ]
  try {
    await execFileAsync("bash", [join(repositoryRoot, "docker/publication/entrypoint.sh"), "true"], {
      env: {
        ...publicationEntrypointEnvironment({ root, home, bindings }),
        CHARIOX_PUBLICATION_PROVIDER_ACCOUNT_BINDINGS: JSON.stringify({
          schema_version: 1,
          defaults: [{ provider: "codex", account_profile: "profile-main" }],
          accounts,
        }),
      },
    })
    assert.equal(await readFile(join(home, ".codex", "auth.json"), "utf8"), "profile-main")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication entrypoint rejects unsafe credential profile and destination paths", async () => {
  const cases = [
    {
      name: "source symlink",
      expected: /credential profile contains an unsafe path/,
      setup: async ({ root, bindings }) => {
        const codex = join(bindings, "000", "home", ".codex")
        await mkdir(codex, { recursive: true })
        await writeFile(join(root, "outside-auth.json"), "outside")
        await symlink(join(root, "outside-auth.json"), join(codex, "auth.json"))
      },
    },
    {
      name: "bindings root symlink",
      expected: /credential bindings root must not be a symlink/,
      setup: async ({ root, bindings }) => {
        const target = join(root, "bindings-target")
        const codex = join(target, "000", "home", ".codex")
        await mkdir(codex, { recursive: true })
        await writeFile(join(codex, "auth.json"), "codex-fixture")
        await symlink(target, bindings)
      },
    },
    {
      name: "unsupported home entry",
      expected: /credential profile contains an unsupported home entry/,
      setup: async ({ bindings }) => {
        const ssh = join(bindings, "000", "home", ".ssh")
        await mkdir(ssh, { recursive: true })
        await writeFile(join(ssh, "id"), "not-a-provider-profile")
      },
    },
    {
      name: "destination symlink",
      expected: /credential destination contains an unsafe path/,
      setup: async ({ home, bindings }) => {
        const escape = join(home, "escape")
        const codex = join(bindings, "000", "home", ".codex")
        await mkdir(escape, { recursive: true })
        await symlink(escape, join(home, ".codex"))
        await mkdir(codex, { recursive: true })
        await writeFile(join(codex, "auth.json"), "codex-fixture")
      },
    },
  ]

  for (const fixture of cases) {
    const root = await mkdtemp(join(tmpdir(), "chariox-publication-unsafe-credentials-"))
    const home = join(root, "home")
    const bindings = join(root, "bindings")
    await mkdir(home, { recursive: true })
    try {
      await fixture.setup({ root, home, bindings })
      await assert.rejects(
        execFileAsync("bash", [join(repositoryRoot, "docker/publication/entrypoint.sh"), "true"], {
          env: publicationEntrypointEnvironment({ root, home, bindings }),
        }),
        (error) => {
          assert.equal(error.code, 70, fixture.name)
          assert.match(error.stderr, fixture.expected, fixture.name)
          return true
        },
      )
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  }
})

function publicationEntrypointEnvironment({ root, home, bindings }) {
  return {
    ...process.env,
    HOME: home,
    CHARIOX_CONFIG_DIR: join(home, ".config", "chariox"),
    CHARIOX_DATA_DIR: join(home, ".local", "share", "chariox"),
    CHARIOX_RUNTIME_DIR: join(home, ".cache", "chariox", "runtime"),
    CHARIOX_SESSION_HISTORY_DIR: join(home, ".local", "share", "chariox", "sessions"),
    CHARIOX_CREDENTIAL_BINDINGS_ROOT: bindings,
    CHARIOX_PROVIDER_CREDENTIALS_DIR: join(root, "missing-legacy-profile"),
    CHARIOX_WORKSPACE_DIR: join(root, "workspace"),
  }
}
