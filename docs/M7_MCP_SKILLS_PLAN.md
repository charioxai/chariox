# M7 MCPs and Skills Plan

M7 adds Arroba-owned MCP and skill management. The design intentionally follows Codex's model where practical, while keeping Arroba responsible for orchestration, per-agent grants, local/remote placement, and provider-specific session rendering.


## Implementation Status

Updated: 2026-04-17

Landed:

- M7.1/M7.2: Arroba-owned MCP config model and registry for project/user roots.
- M7.3 partial: interactive `/mcp list`, `/mcp show`, `/mcp install`, `/mcp grant`, and `/mcp revoke`.
- M7.5 partial: agent model now stores `mcp_grants`; grant/revoke IPC validates installed MCPs before mutating the agent.
- M7.6 partial: local Codex and OpenCode provider launches render only the target agent's granted Arroba MCPs into provider-native MCP config, while keeping Arroba runtime MCP separate.
- M7.7/M7.8: Codex-style `SKILL.md` metadata parsing and Arroba-owned skill registry over project/user roots.
- M7.9 partial: interactive `/skill list`, `/skill show`, `/skill install`, `/skill grant`, and `/skill revoke`.
- M7.11 partial: agent model now stores `skill_grants`; grant/revoke IPC validates installed skills before mutating the agent.
- M7.12 partial: local provider prompts receive a short granted-skills summary for the target agent only. Stored prompt history remains the original user prompt.

Still open in M7:

- Regular non-interactive `arroba mcp ...` / `arroba skill ...` CLI command surfaces, if we keep them separate from slash commands.
- MCP/skill update and uninstall operations.
- Agent grant inspection commands, for example `agent mcps` / `agent skills` or `/mcp grants`.
- Provider MCP import from Codex/OpenCode/Claude-owned configs.
- Provider skill import from Codex/OpenCode/Claude-owned skill locations.
- Explicit full `SKILL.md` body injection when a granted skill is selected, mentioned, or requested.
- Skill MCP dependency validation.
- Runtime MCP discovery/request tools for MCPs and skills.
- Remote-machine MCP and skill materialization/rendering.
- Local and remote drills.

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

Status: partial. Install, list, and show are landed; update and uninstall remain open.

Implement registry operations over Arroba-owned MCP roots:

- install
- list
- show
- update
- uninstall

Registry entries are Arroba-owned copies. Installing an MCP registers it; it does not expose it to every agent.

## M7.3 MCP CLI Commands

Status: partial. Interactive slash commands for install/list/show/grant/revoke are landed; regular command aliases, import, update, uninstall, grant inspection, and test/start remain open.

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
- import MCPs from provider
- list installed MCPs
- inspect MCP config
- grant MCP to agent
- revoke MCP from agent
- show agent MCP grants
- test/start MCP server where feasible

## M7.4 Provider MCP Import

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
- handle name collisions with prompt/rename/skip CLI behavior
- imported MCPs become Arroba-owned copies

## M7.5 Agent MCP Grants

Status: partial. Agent-scoped MCP grant storage and grant/revoke IPC are landed; grant inspection commands remain open.

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

Status: partial. Install, list, and show are landed; update and uninstall remain open.

Implement registry operations over Arroba-owned skill roots:

- install
- list
- show
- update
- uninstall

Arroba-managed skills are not stored in `.agents/skills` by default because providers may auto-scan those paths and expose them outside Arroba's per-agent grants.

## M7.9 Skill CLI Commands

Status: partial. Interactive slash commands for install/list/show/grant/revoke are landed; regular command aliases, import, update, uninstall, grant inspection, and validation commands remain open.

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
- import skills from provider
- list installed skills
- inspect skill metadata/body
- grant skill to agent
- revoke skill from agent
- show agent skill grants
- validate skill dependencies

## M7.10 Provider Skill Import

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

## M7.11 Agent Skill Grants

Status: partial. Agent-scoped skill grant storage and grant/revoke IPC are landed; grant inspection commands remain open.

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

Status: open.

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

Status: open.

Extend Arroba's runtime MCP with discovery/request tools:

```text
list_mcps
list_skills
request_mcp
request_skill
```

For now, assume permission is granted. Later this plugs into the permissions model.

## M7.16 Workflow Integration

Workflows do not override agent grants.

A workflow node using an agent receives that agent's MCPs and skills. If a workflow needs a specific MCP or skill, the user should grant it to the relevant agent.

## M7.17 Remote Machine MCP Support

Status: open.

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

- make Arroba-owned skills available to remote worker kernels
- preserve skill directory layout, including `SKILL.md`, assets, scripts, and references
- avoid provider-scanned paths unless explicitly importing provider-native skills
- inject granted skill summaries/full bodies into the remote agent prompt from Arroba-controlled skill copies
- validate script/reference paths resolve on the worker if a skill needs local files

Design details are deferred until local MCP/skill support is stable.

## M7.19 Local Provider Drills

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

## M7.20 Remote Provider Drills

After remote support is designed and implemented:

- remote agent receives only its granted MCPs
- remote agent receives only its granted skills
- missing remote command/env validation works
- remote skill files are available and readable
- home/local and remote agents with different grants do not leak tools/skills to each other
