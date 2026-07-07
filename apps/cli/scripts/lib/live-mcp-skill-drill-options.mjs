const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000
const WEB_SKILL_REPO = 'https://github.com/vercel-labs/agent-skills.git'

export function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    historyDir: null,
    keepArtifactsOnFailure: false,
    skipLiveProvider: false,
    liveMcpUse: false,
    requireWebSkill: false,
    includeGithubMcp: false,
    webSkillRepo: WEB_SKILL_REPO,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--history-dir') options.historyDir = argv[++i]
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--skip-live-provider') options.skipLiveProvider = true
    else if (arg === '--live-mcp-use') options.liveMcpUse = true
    else if (arg === '--skip-live-mcp-use') options.liveMcpUse = false
    else if (arg === '--require-web-skill') options.requireWebSkill = true
    else if (arg === '--include-github-mcp') options.includeGithubMcp = true
    else if (arg === '--web-skill-repo') options.webSkillRepo = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

export function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-mcp-skill-drill.mjs [options]',
    '',
    'Runs a local M7 MCP/skill drill with isolated daemon/session/workspace lifecycle:',
    '- installs MCPs into the Arroba registry, including Playwright and a deterministic local echo MCP by default',
    '- optionally installs GitHub MCP when --include-github-mcp is set and a GitHub token env var exists',
    '- installs a public web skill repo into an isolated Arroba skill root when reachable',
    '- verifies per-agent MCP/skill grants and same-turn skill request bodies',
    '- optionally prompts local Codex/OpenCode agents to use granted skills and Playwright MCP',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=opencode/gpt-5.2)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --history-dir PATH (required when using --no-spawn-daemon and live provider checks)',
    '  --keep-artifacts-on-failure',
    '  --skip-live-provider (registry/runtime checks only)',
    '  --live-mcp-use (also require a live provider-native Playwright tool call after relaunching the provider run with granted MCPs)',
    '  --require-web-skill (fail if the public skill repo cannot be cloned/imported)',
    '  --include-github-mcp (install GitHub MCP when GITHUB_PERSONAL_ACCESS_TOKEN or GITHUB_TOKEN is set)',
    `  --web-skill-repo ${WEB_SKILL_REPO}`,
  ].join('\n'))
}
