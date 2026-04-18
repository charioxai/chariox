# M7 MCPs and Skills Plan

M7 adds Arroba-owned MCP and skill management. The design intentionally follows Codex's model where practical, while keeping Arroba responsible for orchestration, per-agent grants, local/remote placement, and provider-specific session rendering.


## Implementation Status

Updated: 2026-04-18

Landed:

- M7.1/M7.2: Arroba-owned MCP config model and registry for project/user roots, including update and uninstall lifecycle operations.
- M7.3 partial: interactive `/mcp list`, `/mcp show`, `/mcp install`, `/mcp update`, `/mcp uninstall`, `/mcp import`, `/mcp grant`, `/mcp revoke`, and `/mcp grants`.
- M7.4 partial: `/mcp import codex [name]` and `/mcp import opencode [name]` import supported provider MCP config entries into Arroba-owned MCP registry roots and report skipped entries.
- M7.5 partial: agent model now stores `mcp_grants`; grant/revoke IPC validates installed MCPs before mutating the agent; interactive grant inspection is landed.
- M7.6 partial: local Codex and OpenCode provider launches render only the target agent's granted Arroba MCPs into provider-native MCP config, while keeping Arroba runtime MCP separate.
- M7.7/M7.8: Codex-style `SKILL.md` metadata parsing and Arroba-owned skill registry over project/user roots, including update and uninstall lifecycle operations.
- M7.9 partial: interactive `/skill list`, `/skill show`, `/skill install`, `/skill update`, `/skill uninstall`, `/skill import codex|opencode [name]`, `/skill grant`, `/skill revoke`, and `/skill grants`.
- M7.11 partial: agent model now stores `skill_grants`; grant/revoke IPC validates installed skills before mutating the agent; interactive grant inspection is landed.
- M7.12 partial: local provider prompts receive a short granted-skills summary for the target agent only. Stored prompt history remains the original user prompt.
- M7.13 partial: local provider prompts inject the full `SKILL.md` body for granted skills that are explicitly selected, mentioned, or requested.
- M7.15 partial: runtime MCP exposes `list_capabilities` and `request_capability` control-plane tools for Arroba-managed MCPs and skills. V1 auto-grants valid requests to the current agent and reports when the grant becomes effective. Skill requests now return the full `SKILL.md` body by default, so requested skills can be used in the same turn. Remote worker agents now forward capability discovery/request calls to the home kernel; skill requests return a home-packaged skill directory and materialize it on the worker under `.arroba/remote/skills/<home-kernel-id>/<skill>/<version>/`.
- M7.18: remote skill packaging/materialization is landed for grant-time sync and same-turn `request_capability` use, preserving `SKILL.md`, assets, scripts, and references while skipping provider/cache/build directories and symlinks. Remote prompt dispatch verifies/synchronizes granted skills before submit and injects worker-local `materialized_root` paths into the prompt context.
- M7.20 partial: remote skill live drills pass for OpenCode with `openai/gpt-5.2` low effort and Codex with `gpt-5.2` low effort.

Still open in M7:

- Regular non-interactive `arroba mcp ...` / `arroba skill ...` CLI command surfaces, if we keep them separate from slash commands.
- Regular non-interactive agent grant inspection commands, for example `arroba agent mcps` / `arroba agent skills`.
- Provider MCP import from Claude-owned configs, plus regular non-interactive Codex/OpenCode import aliases.
- Provider skill import from Claude-owned skill locations, plus regular non-interactive Codex/OpenCode import aliases.
- Skill MCP dependency validation.
- Remote-machine MCP materialization/rendering.
- Remote MCP materialization/rendering.
- Local and remote drills.
- MCP provider hot reload. Codex and OpenCode both expose provider-side reload mechanisms, but v1 keeps newly requested MCPs as next-provider-launch because the available reload paths are provider/server scoped rather than safely Arroba agent-scoped.

## Goal

Users should install and manage MCPs and skills through Arroba instead of repeating the same setup in every provider. Agents should receive only the MCPs and skills that were granted to them.

Arroba is an OS/orchestrator, not a provider harness. Third-party MCPs are exposed through each provider's native MCP support. Arroba's runtime MCP remains reserved for Arroba-owned runtime features such as managed I/O, workflow tools, and capability discovery/request control-plane operations.

## Storage Roots

Arroba-managed MCPs and skills must live outside provider-scanned locations to avoid duplicate exposure and context pollution.

Project-local roots:

```text
./.arroba/mcps
./.arroba/skills
```

User-global roots:

```text
~/.arroba/mcps
~/.arroba/skills
```

Provider-native MCPs and skills remain provider-owned. Import commands copy or register provider definitions into Arroba-owned roots; they do not mutate provider config.

## M7.1 Arroba MCP Config Model

Status: landed for V1 stdio and streamable HTTP config.

Define an Arroba-owned MCP config model aligned with Codex where practical:

- stdio transport: command, args, env, env var pass-through, cwd
- streamable HTTP transport: URL, bearer-token env var, headers, env-backed headers
- enabled flag
- required flag
- startup timeout
- tool timeout
- enabled tools
- disabled tools
- per-tool config where useful

V1 should store env var names instead of secret values by default.

## M7.2 Arroba MCP Registry

Status: landed for install, list, show, update, and uninstall.

Implement registry operations over Arroba-owned MCP roots:

- install
- list
- show
- update
- uninstall

Registry entries are Arroba-owned copies. Installing an MCP registers it; it does not expose it to every agent.

## M7.3 MCP CLI Commands

Status: partial. Interactive slash commands for install/list/show/update/uninstall/grant/revoke/grants and Codex/OpenCode import are landed; regular command aliases, Claude import, and test/start remain open.

Expose MCP management through regular CLI commands and the interactive slash-command surface.

Regular commands:

```bash
arroba mcp install browser --command npx --arg @playwright/mcp@latest
arroba mcp list
arroba mcp show browser
arroba mcp update browser ...
arroba mcp uninstall browser
```

Interactive command:

```text
/mcp
```

Expected `/mcp` actions:

- install MCP
- update MCP
- uninstall MCP
- import MCPs from provider
- list installed MCPs
- inspect MCP config
- revoke MCP from agent
- show agent MCP grants
- test/start MCP server where feasible

## M7.4 Provider MCP Import

Status: partial. Codex and OpenCode MCP import are landed through `/mcp import codex [name]` and `/mcp import opencode [name]`. Claude import and regular command aliases remain open.

Add provider import commands so users can reuse existing provider installs without reinstalling manually:

```bash
arroba codex import mcps
arroba codex import mcp browser
arroba opencode import mcps
arroba opencode import mcp github
arroba claude import mcps
```

Behavior:

- read provider-native MCP config
- convert supported definitions into Arroba MCP config
- mark unsupported fields explicitly
- do not delete or mutate provider config
- handle name collisions with prompt/rename/skip CLI behavior; current Codex import skips already-installed names
- imported MCPs become Arroba-owned copies

Current Codex import notes:

- reads `CODEX_HOME/config.toml` when `CODEX_HOME` is set, otherwise `~/.codex/config.toml`
- imports stdio and streamable HTTP MCPs
- preserves command, args, cwd, env, env vars, URL, bearer-token env var, HTTP headers, enabled/required flags, timeouts, and tool allow/deny lists
- refuses inline `bearer_token` secrets and unsupported Codex-specific fields such as OAuth scopes/resources

Current OpenCode import notes:

- reads `OPENCODE_CONFIG` when set, `OPENCODE_CONFIG_DIR`, project `opencode.jsonc`/`opencode.json`, project `.opencode/opencode.jsonc`/`opencode.json`, and global XDG config roots
- imports local MCPs from OpenCode `command` arrays into Arroba stdio MCPs
- imports remote MCPs into Arroba streamable HTTP MCPs
- maps `{env:VAR}` header values into env-backed HTTP headers
- maps local environment entries of the form `"VAR": "{env:VAR}"` into stdio env-var passthrough
- skips OpenCode OAuth MCP config for now

## M7.5 Agent MCP Grants

Status: partial. Agent-scoped MCP grant storage, grant/revoke IPC, and `/mcp grants <agent-ref>` are landed; regular non-interactive grant inspection remains open.

Persist per-agent MCP grants. Each agent has an effective MCP set computed from Arroba's registry plus that agent's grants.

Commands:

```bash
arroba agent grant mcp agent-1 browser
arroba agent revoke mcp agent-1 browser
arroba agent mcps agent-1
```

## M7.6 Provider-Native MCP Rendering

Status: partial. Local Codex and OpenCode launches now receive only the target agent's granted Arroba MCPs as provider-native MCP config. Remote rendering remains open.

At agent launch, inject only that agent's granted Arroba MCPs into the provider's native MCP config/session overlay.

The provider sees normal MCP servers. Arroba does not proxy third-party MCPs through Arroba's runtime MCP.

## M7.7 Arroba Skill Format

Status: landed for Codex-style `SKILL.md` metadata parsing with required name/description behavior.

Use Codex-compatible `SKILL.md` layout and metadata where practical:

```text
skill-name/
  SKILL.md
  agents/openai.yaml      optional
  assets/                 optional
  scripts/                optional
  references/             optional
```

`SKILL.md` should include frontmatter with at least a name and description. Arroba should preserve additional files relative to the skill directory.

## M7.8 Arroba Skill Registry

Status: landed for install, list, show, update, and uninstall.

Implement registry operations over Arroba-owned skill roots:

- install
- list
- show
- update
- uninstall

Arroba-managed skills are not stored in `.agents/skills` by default because providers may auto-scan those paths and expose them outside Arroba's per-agent grants.

## M7.9 Skill CLI Commands

Status: partial. Interactive slash commands for install/list/show/update/uninstall/import/grant/revoke/grants are landed; regular command aliases, Claude import, and validation commands remain open.

Expose skill management through regular CLI commands and the interactive slash-command surface.

Regular commands:

```bash
arroba skill install ./path/to/skill
arroba skill list
arroba skill show browser-qa
arroba skill update browser-qa ...
arroba skill uninstall browser-qa
```

Interactive commands:

```text
/skill
/skills
```

Expected `/skill` actions:

- install skill
- update skill
- uninstall skill
- import skills from provider
- list installed skills
- inspect skill metadata/body
- grant skill to agent
- revoke skill from agent
- show agent skill grants
- validate skill dependencies

## M7.10 Provider Skill Import

Status: partial. Codex and OpenCode skill import are landed through `/skill import codex [name]` and `/skill import opencode [name]`. Claude import and regular command aliases remain open.

Add provider import commands so users can reuse existing provider skills:

```bash
arroba codex import skills
arroba codex import skill browser-qa
arroba opencode import skills
arroba opencode import skill browser-qa
arroba claude import skills
```

Behavior:

- read provider-native skill locations/config
- copy or register supported skills into Arroba-owned skill roots
- preserve `SKILL.md`, assets, scripts, references, and metadata
- do not mutate provider-owned skill dirs
- handle name collisions with prompt/rename/skip CLI behavior
- imported skills become Arroba-owned copies

Current Codex import notes:

- scans project `.codex/skills`, project `.agents/skills`, `CODEX_HOME/skills` or `~/.codex/skills`, and `~/.agents/skills`
- skips Codex cached `.system` skills
- copies the full skill directory into Arroba-owned skill roots

Current OpenCode import notes:

- scans project `.opencode/skill`, project `.opencode/skills`, project `.agents/skills`, `~/.agents/skills`, global OpenCode skill roots, and `skills.paths` entries from OpenCode config
- copies the full skill directory into Arroba-owned skill roots
- URL-backed OpenCode skill discovery/import is not implemented yet

## M7.11 Agent Skill Grants

Status: partial. Agent-scoped skill grant storage, grant/revoke IPC, and `/skill grants <agent-ref>` are landed; regular non-interactive grant inspection remains open.

Persist per-agent skill grants. Each agent has an effective skill set computed from Arroba's registry plus that agent's grants.

Commands:

```bash
arroba agent grant skill agent-1 browser-qa
arroba agent revoke skill agent-1 browser-qa
arroba agent skills agent-1
```

## M7.12 Skill Prompt Injection

Status: partial. Local provider prompts receive target-agent granted skill summaries. Remote prompts and richer selection behavior remain open.

Inject a short Codex-style available-skills section into every relevant agent prompt/run, listing only that agent's granted Arroba skills.

Do not inject all full skill bodies by default.

## M7.13 Explicit Skill Body Injection

Status: partial. Local provider prompts now inject the full `SKILL.md` body for granted skills that are explicitly selected, mentioned, or requested. Remote prompt dispatch and richer explicit-selection UI remain open.

When a granted skill is explicitly selected, mentioned, or requested, inject the full `SKILL.md` body Codex-style.

The runtime MCP may be used for discovery/request control-plane operations, but actual skill instruction exposure is prompt injection.

## M7.14 Skill MCP Dependency Validation

Status: open.

If an Arroba skill declares MCP dependencies, validate that the same agent has those MCPs installed and granted.

For v1:

- do not auto-install missing dependencies
- do not mutate provider global config
- report missing dependencies clearly

## M7.15 Runtime MCP Discovery/Request Control Plane

Status: partial. Local agent discovery and auto-grant requests are landed through the Arroba runtime MCP. Same-turn skill request use is landed by returning the requested `SKILL.md` body. Remote worker agents forward discovery/request calls to the home kernel; skill requests transfer and materialize the complete skill directory on the worker.

Extend Arroba's runtime MCP with discovery/request tools:

```text
list_capabilities
request_capability
```

`list_capabilities` returns Arroba-managed MCPs and skills visible from the current workspace, plus whether each one is already granted to the current agent. `request_capability` accepts `kind` (`mcp` or `skill`) and `name`; for v1, valid requests are auto-granted.

Effectiveness semantics:

- MCP requests update the agent grant immediately, but provider-native MCP exposure is rendered at provider launch, so the agent must restart/relaunch its provider run before using a newly granted MCP.
- Skill requests update the agent grant immediately and return the full `SKILL.md` body by default. The current turn can follow that body immediately, and later turns also receive normal prompt injection for granted/selected skills.
- Remote skill requests are authorized against the home agent/session and package the home skill directory for the worker. The worker writes the package atomically under `.arroba/remote/skills/<home-kernel-id>/<skill>/<version>/`, verifies file hashes, and adds `materialized_root`, `version_hash`, and file paths to the tool result.

Later this plugs into the permissions model instead of always granting.

Hot reload investigation:

- OpenCode has dynamic server-level MCP add/connect routes and rebuilds MCP tools from currently connected MCP clients when constructing prompt tools. That is close to hot reload, but the OpenCode MCP state is server-global. Using it for Arroba per-agent requests would leak a newly requested MCP to other OpenCode sessions/agents served by that OpenCode process unless Arroba first isolates one OpenCode server/process per agent grant scope.
- Codex app-server documents `config/mcpServer/reload`, which reloads MCP server config from disk and queues a refresh for loaded threads on each thread's next active turn. Arroba currently injects granted MCPs through provider launch config, not by mutating Codex's user config on disk. Using this reload safely would require an Arroba-owned, agent-scoped Codex config overlay or a thread-scoped reload API.
- V1 decision: do not implement MCP hot reload yet. Skill hot reload is safe because Arroba can return inert markdown instructions directly through its runtime MCP. MCP hot reload remains a follow-up after provider process/config isolation is explicit.

## M7.16 Workflow Integration

Workflows do not override agent grants.

A workflow node using an agent receives that agent's MCPs and skills. If a workflow needs a specific MCP or skill, the user should grant it to the relevant agent.

## M7.17 Remote Machine MCP Support

Status: landed for skills. Remote MCP support remains separate in M7.17.

Add remote-machine handling for MCPs.

Initial placeholder:

- track where each agent runs: home/local kernel or remote worker kernel
- decide how Arroba MCP registry entries are materialized on remote machines
- resolve stdio command availability on the worker machine
- pass env var names, not secret values
- validate missing commands/env vars on the worker before launch
- inject MCP config into the remote provider session, not the home provider session
- keep third-party MCPs provider-native on the machine where the agent runs

Design details are deferred until local MCP/skill support is stable.

## M7.18 Remote Machine Skill Support

Status: open.

Add remote-machine handling for skills.

Initial placeholder:

- make Arroba-owned skills available to remote worker kernels through home-kernel packaging
- preserve skill directory layout, including `SKILL.md`, assets, scripts, and references
- avoid provider-scanned paths unless explicitly importing provider-native skills
- inject granted skill summaries/full bodies into the remote agent prompt from Arroba-controlled skill copies
- validate script/reference paths resolve on the worker if a skill needs local files

Current implementation:

- synchronizes granted remote-agent skills at grant time
- synchronizes existing grants when a home agent is bound to a remote leased worker
- verifies and repairs remote skill synchronization before prompt submit
- packages whole skill directories from the home registry when a remote worker agent requests a skill through `request_capability`
- skips symlinks and heavy/provider/cache directories such as `.git`, `node_modules`, `.venv`, `target`, `dist`, and `build`
- verifies per-file SHA-256 and package version hashes during worker materialization
- keeps materialized copies under `.arroba/remote/skills/...`, not provider-scanned skill roots
- injects remote prompt skill context with each skill's worker-local `materialized_root`

Open:

- decide whether skill scripts are allowed to run from materialized paths and how to report missing worker-side runtime dependencies

## M7.19 Local Provider Drills

Status: local pass. The local MCP/skill drill harness is landed as `apps/cli/scripts/live-mcp-skill-drill.mjs`; registry, public web skill install, pre-granted skill use, same-turn skill body requests, and provider-native Playwright MCP use pass for local Codex and OpenCode. The strict `--live-mcp-use` path grants Playwright to a fresh MCP drill agent, force-restarts an idle provider process so next-launch MCP grants are rendered, relaunches the provider run, verifies a provider-native Playwright/browser tool call, and writes the marker through Arroba managed I/O. Remote MCP/skill materialization remains open.

Verify local Codex/OpenCode behavior:

- Agent A sees only its granted MCPs.
- Agent B sees only its granted MCPs.
- Agent A sees only its granted skills in prompt context.
- Explicit skill use injects the right `SKILL.md`.
- `/mcp` can install/import/list/grant/revoke.
- `/skill` or `/skills` can install/import/list/grant/revoke.
- Provider-native imported MCPs work after import.
- Provider-native imported skills work after import.
- No duplicate exposure from provider-scanned skill paths.

Overarching local drill matrix:

- Registry lifecycle: install one real stdio MCP, install one HTTP MCP when a local server fixture is available, install one public GitHub skill into an isolated `.arroba/skills` drill workspace, list/show/update/uninstall, and verify files are cleaned on success.
- Per-agent MCP isolation: create two local agents in one session; grant a real MCP such as Playwright/browser only to Agent A; verify Agent A can see/use the MCP after provider relaunch and Agent B cannot see those tools.
- Skill same-turn request: create an agent without a skill grant, prompt it to call `list_capabilities`, request the public skill, and follow the returned `SKILL.md` body in the same response without waiting for another prompt.
- Skill prompt injection: grant the public skill before launch, then verify the provider prompt exposes only the granted skill summary and injects the full body only when explicitly selected/mentioned/requested.
- Real web-skill install sources: use public Agent Skills repositories such as <https://github.com/vercel-labs/agent-skills> or curated registries such as <https://github.com/gotalab/skillport> as install candidates, copying into the isolated Arroba skill root rather than provider-scanned `.agents/skills`.
- Provider import: seed Codex/OpenCode native MCP and skill locations with a fixture, import into Arroba-owned roots, then verify grants/rendering come from Arroba-owned copies and not provider-scanned duplicates.
- Recovery/request UX: when an agent cannot see a needed MCP, it should request it and receive an explicit `next_provider_launch` response; when it cannot see a needed skill, it should request it and receive the skill body immediately.

## M7.20 Remote Provider Drills

Status: partial. Remote skill drills pass; remote MCP drills remain open.

- remote agent receives only its granted MCPs
- remote agent receives only its granted skills
- missing remote command/env validation works
- remote skill files are available and readable
- home/local and remote agents with different grants do not leak tools/skills to each other

Remote skill drill:

```bash
node apps/cli/scripts/live-remote-skill-drill.mjs --provider opencode --model openai/gpt-5.2 --effort low
node apps/cli/scripts/live-remote-skill-drill.mjs --provider codex --model gpt-5.2 --effort low
```

Observed on 2026-04-18: both drills passed. The drill creates isolated relay/home/worker daemons, installs an Arroba-owned skill with an asset, spawns a remote leased agent, grants the skill, verifies grant-time worker materialization, submits a live remote prompt, and verifies the provider wrote `outputs/remote-skill-provider.txt` with the asset token and `REMOTE_SKILL_DRILL_OK`.
