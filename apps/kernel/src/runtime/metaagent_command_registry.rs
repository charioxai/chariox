use crate::transport::runtime_tools::{
    MetaCommandDocsArgs, MetaCommandListArgs, MetaCommandSearchArgs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaCommandPolicy {
    Allow,
    Approval,
    Deny,
}

impl MetaCommandPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Approval => "approval",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaCommandDoc {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) usage: &'static str,
    pub(crate) examples: &'static [&'static str],
    pub(crate) tags: &'static [&'static str],
    pub(crate) intents: &'static [&'static str],
    pub(crate) scope: &'static str,
    pub(crate) mutates: bool,
    pub(crate) policy: MetaCommandPolicy,
    pub(crate) authority: &'static str,
    pub(crate) routed: bool,
    pub(crate) description: &'static str,
}

pub(crate) const AGENT_SPAWN_USAGE: &str = "agent spawn [alias] [model] [--provider <provider>] [--model <model>] [--effort <effort>|--variant <variant>] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>] [--slice off|new|new:headless|new:headed|<slice-ref>]";

pub(crate) const META_COMMANDS: &[MetaCommandDoc] = &[
    MetaCommandDoc {
        name: "session overview",
        aliases: &["context", "agent list", "workflow list"],
        usage: "Use arroba.meta.session_overview for current session state.",
        examples: &["arroba.meta.session_overview({})"],
        tags: &["inspect", "session", "agents", "workflows"],
        intents: &[
            "inspect current session",
            "see existing agents",
            "check workflow state",
            "understand what is happening",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "current session",
        routed: true,
        description: "Inspect current session, owned agents, workflow runs, pending interactions, and event counts.",
    },
    MetaCommandDoc {
        name: "prompt",
        aliases: &["prompt <agent-ref> <text>"],
        usage: "prompt <owned-agent-ref> <prompt text>",
        examples: &["prompt agent-2 \"Investigate this failure\""],
        tags: &["prompt", "agent", "orchestration"],
        intents: &[
            "delegate work to an agent",
            "ask a worker to do a task",
            "send instructions to a regular agent",
            "have another agent implement something",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Submit a normal Arroba prompt to one of this user's regular agents through the existing prompt path. Do not use `prompt` to steer an agent that is currently running or queued for workflow work; inspect the workflow run and wait for its events instead. Metaagent prompt commands do not support blocking reply flags; use events, turn_overview, and turn_blob for supervision.",
    },
    MetaCommandDoc {
        name: "agent list",
        aliases: &["agent list", "agent ls"],
        usage: "agent list",
        examples: &["agent list"],
        tags: &["agent", "inspect", "orchestration"],
        intents: &[
            "list agents",
            "see available workers",
            "find existing agents",
            "inspect agent roster",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "owned and visible session agents",
        routed: true,
        description: "List agents in the current session so the metaagent can pick a regular worker to prompt or supervise.",
    },
    MetaCommandDoc {
        name: "agent spawn",
        aliases: &[
            "agent spawn",
            "agent create",
            "spawn agent",
            "create agent",
            "create worker",
            "make worker",
        ],
        usage: AGENT_SPAWN_USAGE,
        examples: &[
            "agent spawn reviewer gpt-5.2",
            "agent spawn verifier --provider opencode --model opencode/gpt-5.2 --effort high",
            "agent spawn verifier --dir ../qa",
            "agent spawn linux-checker --kernel linux-worker",
            "agent spawn isolated-builder --slice new:headless",
            "agent spawn slice-worker --slice linux-dev",
        ],
        tags: &["agent", "spawn", "worker", "orchestration"],
        intents: &[
            "create a new agent",
            "spawn a worker",
            "make a worker agent",
            "delegate implementation to a new agent",
            "start another agent for a task",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned agents only",
        routed: true,
        description: "Create a regular agent owned by the current user. Metaagents cannot spawn another metaagent. Spawn can choose provider, model, and effort/variant, and uses the same placement option contract as normal agent/session launch surfaces: directory placement, remote kernel placement, existing slice placement, and new slice creation.",
    },
    MetaCommandDoc {
        name: "agent focus",
        aliases: &["agent focus"],
        usage: "agent focus <agent-ref>",
        examples: &["agent focus reviewer"],
        tags: &["agent", "focus", "orchestration"],
        intents: &["focus an agent", "switch selected agent", "make an agent current"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned agents only",
        routed: true,
        description: "Focus one of this user's agents in the current session.",
    },
    MetaCommandDoc {
        name: "agent alias",
        aliases: &["agent alias", "agent name"],
        usage: "agent alias <agent-ref> <alias>",
        examples: &["agent alias reviewer code-reviewer"],
        tags: &["agent", "alias", "rename", "orchestration"],
        intents: &["rename an agent", "give an agent an alias", "label a worker"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned agents only",
        routed: true,
        description: "Assign or update a stable alias for one of this user's agents.",
    },
    MetaCommandDoc {
        name: "agent delete",
        aliases: &["agent delete", "agent destroy", "agent remove"],
        usage: "agent delete <agent-ref>",
        examples: &["agent delete code-reviewer"],
        tags: &["agent", "delete", "remove", "orchestration"],
        intents: &["delete an agent", "remove a worker", "clean up an agent"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned agents only",
        routed: true,
        description: "Delete one of this user's regular agents from the current session.",
    },
    MetaCommandDoc {
        name: "workflow list",
        aliases: &["workflow", "workflow list", "workflow ls"],
        usage: "workflow list",
        examples: &["workflow list"],
        tags: &["workflow", "inspect", "orchestration"],
        intents: &["list workflows", "see workflow definitions", "inspect workflow runs"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "List workflow definitions and runs visible in the current session.",
    },
    MetaCommandDoc {
        name: "workflow new",
        aliases: &["workflow new", "workflow create", "create workflow"],
        usage: "workflow new <workflow-name>",
        examples: &["workflow new qa-flow"],
        tags: &["workflow", "create", "orchestration"],
        intents: &["create a workflow", "make an automation graph", "start a workflow plan"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Create a workflow definition that can coordinate regular agents.",
    },
    MetaCommandDoc {
        name: "workflow resolve",
        aliases: &["workflow resolve", "workflow show", "workflow get"],
        usage: "workflow resolve <workflow-ref>",
        examples: &["workflow resolve qa-flow"],
        tags: &["workflow", "inspect", "orchestration"],
        intents: &[
            "inspect workflow definition",
            "get workflow node ids",
            "find workflow endpoint ids",
            "review workflow graph",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Resolve a workflow reference to the full workflow definition, including node, edge, and endpoint ids needed by other workflow commands.",
    },
    MetaCommandDoc {
        name: "workflow alias",
        aliases: &["workflow alias", "workflow name", "rename workflow"],
        usage: "workflow alias <workflow-ref> <alias>",
        examples: &["workflow alias wf_123 qa-flow"],
        tags: &["workflow", "rename", "orchestration"],
        intents: &["rename workflow", "give workflow a stable alias", "label workflow"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Assign or update a human-readable workflow alias.",
    },
    MetaCommandDoc {
        name: "workflow node add",
        aliases: &["workflow node add", "add workflow node"],
        usage: "workflow node add <workflow-ref> <agent-ref>",
        examples: &["workflow node add qa-flow reviewer"],
        tags: &["workflow", "node", "agent", "orchestration"],
        intents: &[
            "add a worker to a workflow",
            "put an agent in a workflow",
            "create a workflow node",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy; regular agents only",
        routed: true,
        description: "Add a regular agent as a node in a workflow. Use the agent alias as the human label; workflow nodes do not take a separate alias. Metaagents cannot be workflow nodes.",
    },
    MetaCommandDoc {
        name: "workflow node remove",
        aliases: &["workflow node remove", "workflow node delete", "remove workflow node"],
        usage: "workflow node remove <workflow-ref> <node-id>",
        examples: &["workflow node remove qa-flow node_123"],
        tags: &["workflow", "node", "remove", "orchestration"],
        intents: &["remove workflow node", "delete workflow node", "take agent out of workflow"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Remove a workflow node by node id. Resolve or list the workflow first if you need node ids.",
    },
    MetaCommandDoc {
        name: "workflow node instructions",
        aliases: &["workflow node instructions", "workflow node instruct"],
        usage: "workflow node instructions <workflow-ref> <node-id> [instructions]",
        examples: &["workflow node instructions qa-flow node_123 \"Review implementation and report risks\""],
        tags: &["workflow", "node", "instructions", "orchestration"],
        intents: &["set node instructions", "describe node task", "change worker instructions"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Set or clear per-node workflow instructions. Use `clear`, `none`, or `-` as the instructions value to clear them.",
    },
    MetaCommandDoc {
        name: "workflow node can-complete",
        aliases: &["workflow node can-complete", "workflow node complete"],
        usage: "workflow node can-complete <workflow-ref> <node-id> <true|false>",
        examples: &["workflow node can-complete qa-flow node_123 true"],
        tags: &["workflow", "node", "completion", "orchestration"],
        intents: &["allow node to complete workflow", "set final node", "prevent node completion"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Control whether a node may emit final workflow output and complete the run.",
    },
    MetaCommandDoc {
        name: "workflow node intermediate-output",
        aliases: &["workflow node intermediate-output", "workflow node intermediate"],
        usage: "workflow node intermediate-output <workflow-ref> <node-id> <true|false>",
        examples: &["workflow node intermediate-output qa-flow node_123 true"],
        tags: &["workflow", "node", "output", "orchestration"],
        intents: &["allow intermediate output", "publish partial workflow output", "disable intermediate output"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Control whether a node may emit intermediate workflow output while the run continues.",
    },
    MetaCommandDoc {
        name: "workflow node wait-for-all-inputs",
        aliases: &[
            "workflow node wait-for-all-inputs",
            "workflow node wait-all",
            "workflow node join",
        ],
        usage: "workflow node wait-for-all-inputs <workflow-ref> <node-id> <true|false>",
        examples: &["workflow node wait-for-all-inputs qa-flow node_123 true"],
        tags: &["workflow", "node", "join", "inputs", "orchestration"],
        intents: &[
            "wait for all workflow inputs",
            "join parallel workflow branches",
            "synchronize workflow inputs",
            "combine branch outputs by iteration",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Control whether a node waits until every incoming edge has an unconsumed handoff from the same source-node iteration before it runs. Use this for join nodes that combine parallel branches.",
    },
    MetaCommandDoc {
        name: "workflow node intermediate-output-schema",
        aliases: &[
            "workflow node intermediate-output-schema",
            "workflow node intermediate schema",
        ],
        usage: "workflow node intermediate-output-schema <workflow-ref> <node-id> <schema-ref|none>",
        examples: &[],
        tags: &["workflow", "node", "schema", "output", "orchestration"],
        intents: &[
            "set intermediate output schema",
            "validate partial workflow output",
            "clear intermediate output schema",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: false,
        description: "Documented workflow capability, not routed through metaagent run_command yet. Use worker instructions and available workflow output tools unless schema routing is explicitly added.",
    },
    MetaCommandDoc {
        name: "workflow node max-turns",
        aliases: &["workflow node max-turns", "workflow node turn limit"],
        usage: "workflow node max-turns <workflow-ref> <node-id> <number|none>",
        examples: &["workflow node max-turns qa-flow node_123 3"],
        tags: &["workflow", "node", "limit", "orchestration"],
        intents: &["limit node turns", "set max turns", "clear node turn limit"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Set or clear the maximum number of turns a workflow node may take in one run.",
    },
    MetaCommandDoc {
        name: "workflow endpoint new",
        aliases: &["workflow endpoint new", "workflow endpoint create"],
        usage: "workflow endpoint new <workflow-ref> <node-ref> <endpoint-name>",
        examples: &["workflow endpoint new qa-flow node-1 default"],
        tags: &["workflow", "endpoint", "orchestration"],
        intents: &["create workflow endpoint", "make workflow entrypoint", "wire workflow input"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Create an endpoint that can start a workflow run.",
    },
    MetaCommandDoc {
        name: "workflow endpoint alias",
        aliases: &["workflow endpoint alias", "workflow endpoint name", "rename workflow endpoint"],
        usage: "workflow endpoint alias <workflow-ref> <endpoint-ref> <alias>",
        examples: &["workflow endpoint alias qa-flow endpoint_123 start"],
        tags: &["workflow", "endpoint", "rename", "orchestration"],
        intents: &["rename endpoint", "label workflow entrypoint", "give endpoint alias"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Assign or update a human-readable alias for a workflow endpoint.",
    },
    MetaCommandDoc {
        name: "workflow endpoint bind",
        aliases: &["workflow endpoint bind"],
        usage: "workflow endpoint bind <workflow-ref> <endpoint-ref> <queue-ref>",
        examples: &[],
        tags: &["workflow", "endpoint", "queue", "orchestration"],
        intents: &[
            "bind workflow endpoint",
            "connect endpoint to queue",
            "attach workflow endpoint",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: false,
        description: "Documented daemon capability, not routed through metaagent run_command yet. For ordinary metaagent workflow runs, create an endpoint and invoke it directly with `workflow run`.",
    },
    MetaCommandDoc {
        name: "workflow endpoint remove",
        aliases: &["workflow endpoint remove", "workflow endpoint delete"],
        usage: "workflow endpoint remove <workflow-ref> <endpoint-ref>",
        examples: &[],
        tags: &["workflow", "endpoint", "remove", "orchestration"],
        intents: &["remove workflow endpoint", "delete workflow endpoint"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "not implemented",
        routed: false,
        description: "Documented as unavailable for metaagents: endpoint removal is not currently exposed through the kernel command router. Prefer aliasing or creating a new endpoint.",
    },
    MetaCommandDoc {
        name: "workflow edge add",
        aliases: &[
            "workflow edge add",
            "add workflow edge",
            "connect workflow nodes",
            "connect nodes",
        ],
        usage: "workflow edge add <workflow-ref> <from-node-id> <to-node-id>",
        examples: &["workflow edge add qa-flow node_a node_b"],
        tags: &["workflow", "edge", "connect", "orchestration"],
        intents: &[
            "connect workflow nodes",
            "add edge between nodes",
            "make workflow sequential",
            "send output from one node to another",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Connect two workflow nodes so output can flow from the source node to the target node.",
    },
    MetaCommandDoc {
        name: "workflow edge remove",
        aliases: &["workflow edge remove", "workflow edge delete", "remove workflow edge"],
        usage: "workflow edge remove <workflow-ref> <edge-id>",
        examples: &["workflow edge remove qa-flow edge_123"],
        tags: &["workflow", "edge", "remove", "orchestration"],
        intents: &["remove workflow edge", "disconnect workflow nodes", "delete edge"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Remove an edge from a workflow by edge id. Resolve the workflow first if you need edge ids.",
    },
    MetaCommandDoc {
        name: "workflow run",
        aliases: &[
            "workflow run",
            "workflow start",
            "run workflow",
            "start workflow",
        ],
        usage: "workflow run <workflow-ref> <endpoint-ref> <prompt>",
        examples: &["workflow run qa-flow default \"Run QA\""],
        tags: &["workflow", "run", "orchestration"],
        intents: &[
            "run a workflow",
            "start a workflow",
            "execute an automation graph",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Start a workflow run from an endpoint with a prompt.",
    },
    MetaCommandDoc {
        name: "workflow runs",
        aliases: &["workflow runs"],
        usage: "workflow runs [workflow-ref]",
        examples: &["workflow runs qa-flow"],
        tags: &["workflow", "inspect", "runs", "orchestration"],
        intents: &["list workflow runs", "inspect workflow run status", "check workflow progress"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "List workflow runs and their current status.",
    },
    MetaCommandDoc {
        name: "workflow get-run",
        aliases: &["workflow get-run", "workflow run-status", "workflow run get"],
        usage: "workflow get-run <run-ref>",
        examples: &["workflow get-run run_123"],
        tags: &["workflow", "inspect", "runs", "orchestration"],
        intents: &["inspect workflow run", "check workflow run details", "read workflow output"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Fetch a single workflow run and its detailed status/output by run reference.",
    },
    MetaCommandDoc {
        name: "workflow tutorial basic",
        aliases: &[
            "workflow guide",
            "workflow tutorial",
            "how to build workflow",
            "workflow example",
        ],
        usage: "Read-only guide entry. Use command_docs(\"workflow tutorial basic\") or search_commands(\"build workflow tutorial\").",
        examples: &[
            "workflow new qa-flow",
            "agent spawn implementer",
            "agent spawn reviewer",
            "workflow node add qa-flow implementer",
            "workflow node add qa-flow reviewer",
            "workflow resolve qa-flow",
            "workflow node instructions qa-flow <implementer-node-id> \"Implement the change, run checks, and hand off files plus command results\"",
            "workflow node instructions qa-flow <reviewer-node-id> \"Review independently, run checks, and complete only on pass\"",
            "workflow node can-complete qa-flow <reviewer-node-id> true",
            "workflow edge add qa-flow <implementer-node-id> <reviewer-node-id>",
            "workflow endpoint new qa-flow <entry-node-id> start",
            "workflow run qa-flow start \"Build and verify the app\"",
            "workflow runs qa-flow",
            "workflow get-run <run-ref>",
        ],
        tags: &["workflow", "guide", "tutorial", "orchestration"],
        intents: &[
            "learn workflow basics",
            "build workflow from scratch",
            "connect nodes and run workflow",
            "create endpoint and trigger workflow",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "documentation only",
        routed: false,
        description: "Basic workflow sequence: create the workflow, spawn regular agents, add each agent as a node, resolve the workflow to get node ids, configure node instructions and completion permissions, connect nodes with edges when work should flow between them, create an endpoint on the entry node, run the endpoint with the task prompt, then inspect runs or a specific run. Configure nodes before running; instruction changes after a run starts do not rewrite the already-dispatched turn. While a run is Running, use `workflow get-run`, events, and turn_overview for visibility. Do not direct-prompt workflow workers, create replacement workflows, or cancel/restart the run unless the run is truly blocked or the user asked to abort. Workflow nodes use agent aliases; node ids come from workflow resolve/list output.",
    },
    MetaCommandDoc {
        name: "workflow run-output-schema",
        aliases: &["workflow run-output-schema", "workflow output schema"],
        usage: "workflow run-output-schema <workflow-ref> <schema-ref|none>",
        examples: &[],
        tags: &["workflow", "schema", "output", "orchestration"],
        intents: &[
            "set workflow final output schema",
            "validate workflow output",
            "clear workflow output schema",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: false,
        description: "Documented workflow capability, not routed through metaagent run_command yet. Use workflow guides and worker output validation until schema routing is added.",
    },
    MetaCommandDoc {
        name: "workflow max-turns",
        aliases: &["workflow max-turns", "workflow turn limit"],
        usage: "workflow max-turns <workflow-ref> <number|none>",
        examples: &[],
        tags: &["workflow", "limit", "orchestration"],
        intents: &["set workflow turn limit", "limit workflow turns", "clear workflow turn limit"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "not implemented",
        routed: false,
        description: "Workflow-level max-turns is not currently exposed as a metaagent command. Use `workflow node max-turns` for per-node limits.",
    },
    MetaCommandDoc {
        name: "workflow node extensions",
        aliases: &["workflow node extensions", "workflow node grant", "workflow node revoke"],
        usage: "workflow node extensions ...",
        examples: &[],
        tags: &["workflow", "node", "extension", "capability"],
        intents: &[
            "grant tool to workflow node",
            "revoke tool from workflow node",
            "configure node extensions",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: false,
        description: "Workflow-node extension commands are not routed directly. Grant MCPs or skills to the owned regular agent with `mcp grant` or `skill grant`; the workflow node inherits the agent capability.",
    },
    MetaCommandDoc {
        name: "workflow pane",
        aliases: &[
            "workflow pane",
            "workflow log",
            "workflow trace",
            "workflow edit",
        ],
        usage: "workflow <pane|log|trace|edit> ...",
        examples: &[],
        tags: &["workflow", "ui", "inspect", "cli-only"],
        intents: &[
            "open workflow pane",
            "view workflow log",
            "inspect workflow trace",
            "edit workflow visually",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Deny,
        authority: "denied",
        routed: false,
        description: "CLI/TUI workflow UI commands are not available to metaagents. Use `workflow resolve`, `workflow runs`, `workflow get-run`, events, and turn_overview instead.",
    },
    MetaCommandDoc {
        name: "workflow cancel",
        aliases: &["workflow cancel"],
        usage: "workflow cancel <run-ref>",
        examples: &["workflow cancel run-1"],
        tags: &["workflow", "cancel", "orchestration"],
        intents: &["cancel a workflow", "stop a workflow run", "abort workflow run"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Cancel an active workflow run.",
    },
    MetaCommandDoc {
        name: "workflow resume",
        aliases: &["workflow resume"],
        usage: "workflow resume <run-ref>",
        examples: &["workflow resume run-1"],
        tags: &["workflow", "resume", "orchestration"],
        intents: &["resume a workflow", "continue a paused workflow run"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
        description: "Resume a paused or resumable workflow run. Do not call this on a Running workflow; poll `workflow get-run`, wait for workflow events, or inspect worker turns instead.",
    },
    MetaCommandDoc {
        name: "agent app guide",
        aliases: &[
            "generate agent app",
            "agent app tutorial",
            "workflow app guide",
            "build web app with agents",
        ],
        usage: "Read-only guide entry. Use command_docs(\"agent app guide\") or search_commands(\"generate agent app\").",
        examples: &[
            "workflow new app-build",
            "agent spawn builder",
            "agent spawn verifier",
            "workflow node add app-build builder",
            "workflow node add app-build verifier",
            "workflow resolve app-build",
            "workflow node instructions app-build <builder-node-id> \"Build the app, run checks, and hand off files plus command results\"",
            "workflow node instructions app-build <verifier-node-id> \"Review independently, run checks, and complete only on pass\"",
            "workflow node can-complete app-build <verifier-node-id> true",
            "workflow edge add app-build <builder-node-id> <verifier-node-id>",
            "workflow endpoint new app-build <builder-node-id> start",
            "workflow run app-build start \"Build the requested app, then verify it\"",
            "workflow get-run <run-ref>",
        ],
        tags: &["guide", "workflow", "agent-app", "webapp", "orchestration"],
        intents: &[
            "generate agent app",
            "build web app with workflow",
            "create app using agents",
            "verify generated application",
        ],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "documentation only",
        routed: false,
        description: "For app-building tasks, use a workflow when requested: create builder and verifier regular agents, add them as workflow nodes, resolve node ids, configure builder/reviewer node instructions, set only the verifier node as able to complete the run, connect builder to verifier, create an endpoint on the builder node, run the workflow with the user's high-level app prompt, then inspect the run and worker turns. The app should be built by the workflow run, not by direct prompts outside the run. While the run is active, wait for final workflow JSON, workflow events, or clear failure before retrying. The metaagent should supervise and complete the task only after the workflow output and worker evidence show the app was built and verified.",
    },
    MetaCommandDoc {
        name: "mcp list",
        aliases: &["mcp", "mcp list", "mcp ls"],
        usage: "mcp list",
        examples: &["mcp list"],
        tags: &["extension", "mcp", "capability", "inspect"],
        intents: &["list mcp tools", "see available mcp servers", "inspect worker tools"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "List MCP definitions known to Arroba.",
    },
    MetaCommandDoc {
        name: "mcp show",
        aliases: &["mcp show", "mcp get"],
        usage: "mcp show <mcp-name>",
        examples: &["mcp show playwright"],
        tags: &["extension", "mcp", "capability", "inspect"],
        intents: &["inspect mcp server", "show mcp config", "check mcp definition"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Inspect one MCP definition by name.",
    },
    MetaCommandDoc {
        name: "mcp install-json",
        aliases: &["mcp install-json", "mcp update-json", "install mcp", "add mcp"],
        usage: "mcp install-json '<mcp-json>'",
        examples: &[
            "mcp install-json '{\"name\":\"playwright\",\"transport\":{\"type\":\"stdio\",\"command\":\"npx\",\"args\":[\"@playwright/mcp\"],\"env\":{},\"env_vars\":[]},\"enabled\":true,\"required\":false}'",
        ],
        tags: &["extension", "mcp", "capability", "install", "tool"],
        intents: &[
            "install an mcp server",
            "add a tool provider",
            "make a new mcp capability available",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Install or update an MCP definition through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "mcp update-json",
        aliases: &["mcp update-json", "update mcp"],
        usage: "mcp update-json '<mcp-json>'",
        examples: &[
            "mcp update-json '{\"name\":\"playwright\",\"transport\":{\"type\":\"stdio\",\"command\":\"npx\",\"args\":[\"@playwright/mcp\"],\"env\":{},\"env_vars\":[]},\"enabled\":true,\"required\":false}'",
        ],
        tags: &["extension", "mcp", "capability", "update", "tool"],
        intents: &["update mcp server", "change mcp config", "replace mcp definition"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Update an existing MCP definition through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "mcp grant",
        aliases: &[
            "mcp grant",
            "mcp revoke",
            "grant mcp",
            "give worker tool",
            "give agent mcp",
        ],
        usage: "mcp grant <agent-ref> <mcp-name>",
        examples: &["mcp grant agent-2 playwright"],
        tags: &["extension", "mcp", "capability", "grant", "tool"],
        intents: &[
            "give a worker an mcp tool",
            "grant a tool to an agent",
            "enable mcp for a regular agent",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Grant or revoke an MCP extension for one of this user's regular agents. Metaagents cannot grant MCP tools to themselves.",
    },
    MetaCommandDoc {
        name: "mcp uninstall",
        aliases: &["mcp uninstall", "mcp remove"],
        usage: "mcp uninstall <mcp-name>",
        examples: &["mcp uninstall playwright"],
        tags: &["extension", "mcp", "capability", "remove", "tool"],
        intents: &["remove mcp server", "uninstall tool provider", "delete mcp capability"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Remove an MCP definition through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "mcp revoke",
        aliases: &["mcp revoke", "revoke mcp", "remove worker mcp"],
        usage: "mcp revoke <agent-ref> <mcp-name>",
        examples: &["mcp revoke agent-2 playwright"],
        tags: &["extension", "mcp", "capability", "revoke", "tool"],
        intents: &["remove tool from worker", "revoke mcp from agent", "disable agent mcp"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Revoke an MCP extension from one of this user's regular agents.",
    },
    MetaCommandDoc {
        name: "mcp import",
        aliases: &[
            "mcp import",
            "import mcp",
            "import provider mcp",
            "sync mcp from provider",
        ],
        usage: "mcp import <codex|opencode|claude> [mcp-name]",
        examples: &[
            "mcp import codex playwright",
            "mcp import opencode",
            "mcp import claude docs",
        ],
        tags: &[
            "extension",
            "mcp",
            "capability",
            "import",
            "provider",
            "tool",
            "discover",
        ],
        intents: &[
            "import mcp from provider config",
            "bring existing mcp into arroba",
            "make a provider mcp grantable",
            "sync local provider mcp tools",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Import MCP definitions from a local provider configuration into the Arroba registry. Only Arroba-registered MCPs can be granted to workers, so list or show MCPs after importing before using `mcp grant`.",
    },
    MetaCommandDoc {
        name: "skill list",
        aliases: &["skill", "skill list", "skills list", "skill ls", "skills"],
        usage: "skill list",
        examples: &["skill list"],
        tags: &["extension", "skill", "capability", "inspect"],
        intents: &["list skills", "see available skills", "inspect worker skills"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "List skills known to Arroba.",
    },
    MetaCommandDoc {
        name: "skill show",
        aliases: &["skill show", "skill get", "skills show", "skills get"],
        usage: "skill show <skill-name>",
        examples: &["skill show browser-qa"],
        tags: &["extension", "skill", "capability", "inspect"],
        intents: &["inspect skill", "show skill config", "check skill definition"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Inspect one skill definition by name.",
    },
    MetaCommandDoc {
        name: "skill install",
        aliases: &[
            "skill install",
            "skill update",
            "install skill",
            "add skill",
        ],
        usage: "skill install <path-or-name>",
        examples: &["skill install ./skills/browser-qa"],
        tags: &["extension", "skill", "capability", "install", "tool"],
        intents: &["install a skill", "add a skill", "make a skill available"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Install or update a skill through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "skill update",
        aliases: &["skill update", "skills update", "update skill"],
        usage: "skill update <path-or-name>",
        examples: &["skill update ./skills/browser-qa"],
        tags: &["extension", "skill", "capability", "update", "tool"],
        intents: &["update skill", "change skill", "replace skill definition"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Update an installed skill through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "skill grant",
        aliases: &[
            "skill grant",
            "skill revoke",
            "grant skill",
            "give worker skill",
            "give agent skill",
        ],
        usage: "skill grant <agent-ref> <skill-name>",
        examples: &["skill grant agent-2 browser-qa"],
        tags: &["extension", "skill", "capability", "grant", "tool"],
        intents: &[
            "give a worker a skill",
            "grant a skill to an agent",
            "enable skill for a regular agent",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Grant or revoke a skill for one of this user's regular agents. Metaagents cannot grant skills to themselves.",
    },
    MetaCommandDoc {
        name: "skill uninstall",
        aliases: &["skill uninstall", "skill remove"],
        usage: "skill uninstall <skill-name>",
        examples: &["skill uninstall browser-qa"],
        tags: &["extension", "skill", "capability", "remove", "tool"],
        intents: &["remove skill", "uninstall skill", "delete worker skill"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Remove a skill through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "skill revoke",
        aliases: &["skill revoke", "revoke skill", "remove worker skill"],
        usage: "skill revoke <agent-ref> <skill-name>",
        examples: &["skill revoke agent-2 browser-qa"],
        tags: &["extension", "skill", "capability", "revoke", "tool"],
        intents: &["remove skill from worker", "revoke skill from agent", "disable agent skill"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Revoke a skill from one of this user's regular agents.",
    },
    MetaCommandDoc {
        name: "skill import",
        aliases: &[
            "skill import",
            "skills import",
            "import skill",
            "import provider skill",
            "sync skill from provider",
        ],
        usage: "skill import <codex|opencode|claude|pi> [skill-name]",
        examples: &[
            "skill import codex browser-qa",
            "skill import opencode",
            "skill import claude docs-helper",
            "skill import pi docs-helper",
        ],
        tags: &[
            "extension",
            "skill",
            "capability",
            "import",
            "provider",
            "tool",
            "discover",
        ],
        intents: &[
            "import skill from provider config",
            "bring existing skill into arroba",
            "make a provider skill grantable",
            "sync local provider skills",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Import skills from local provider skill directories into the Arroba registry. Only Arroba-registered skills can be granted to workers, so list or show skills after importing before using `skill grant`.",
    },
    MetaCommandDoc {
        name: "extension import providers",
        aliases: &[
            "extension import providers",
            "extensions import providers",
            "sync provider capabilities",
            "import all provider capabilities",
            "import provider mcps and skills",
        ],
        usage: "extension import providers [--provider codex|opencode|claude|pi] [--kind all|mcp|skill] [--name <capability>] [--dry-run]",
        examples: &[
            "extension import providers --dry-run",
            "extension import providers --provider codex --provider claude --kind all",
            "extension import providers --provider claude --kind skill --name docs-helper",
            "extension import providers --provider pi --kind skill --name docs-helper",
        ],
        tags: &[
            "extension",
            "mcp",
            "skill",
            "capability",
            "import",
            "provider",
            "sync",
            "dedupe",
            "discover",
        ],
        intents: &[
            "import all provider mcps and skills",
            "sync local provider capabilities into arroba",
            "deduplicate provider tools before granting",
            "make provider capabilities available to grant",
        ],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Discover Codex, OpenCode, and Claude MCPs and skills, deduplicate by capability kind/name/hash, import or update the selected newest definitions in the Arroba registries, and return a compact report. Use this when you are not sure whether a needed MCP or skill is already installed in Arroba; run with `--dry-run` first if you only need discovery.",
    },
    MetaCommandDoc {
        name: "slice",
        aliases: &["slice save", "slice save-state", "slice stop"],
        usage: "slice <list|show|start|stop|save-state|status|backup> ...",
        examples: &["slice save-state dev --restart-agents"],
        tags: &["slice", "environment"],
        intents: &["manage slice", "save slice state", "stop environment"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Deny,
        authority: "denied",
        routed: false,
        description: "Denied for metaagents. Slice placement and management are execution-environment operations, not delegation commands.",
    },
    MetaCommandDoc {
        name: "credential list",
        aliases: &[
            "credential",
            "credential list",
            "credential ls",
            "credentials list",
        ],
        usage: "credential list",
        examples: &["credential list"],
        tags: &["credential", "vault", "sensitive", "inspect"],
        intents: &["list credentials", "see credential handles", "inspect available secrets"],
        scope: "global",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "credential handle metadata only",
        routed: true,
        description: "List credential handles without exposing raw secret values.",
    },
    MetaCommandDoc {
        name: "credential get",
        aliases: &["credential get", "credential show", "credentials get"],
        usage: "credential get <credential-id>",
        examples: &["credential get credential-1"],
        tags: &["credential", "vault", "sensitive", "inspect"],
        intents: &["inspect credential", "view credential metadata", "check credential handle"],
        scope: "global",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "credential handle metadata only",
        routed: true,
        description: "Inspect credential handle metadata without exposing raw secret values.",
    },
    MetaCommandDoc {
        name: "credential upsert-json",
        aliases: &[
            "credential upsert-json",
            "create credential",
            "store credential",
            "add credential handle",
        ],
        usage: "credential upsert-json '<credential-json>'",
        examples: &[
            "credential upsert-json '{\"id\":\"github-token\",\"source\":{\"type\":\"vault\",\"key\":\"github-token\"},\"allowed_uses\":[\"http\"],\"injection\":{\"kind\":\"header\",\"name\":\"authorization\",\"value\":\"Bearer ${secret}\"}}'",
        ],
        tags: &["credential", "vault", "sensitive", "create"],
        intents: &[
            "create credential handle",
            "store credential metadata",
            "add a credential",
            "prepare a secret for worker use",
        ],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "credential handle metadata and vault unlock only",
        routed: true,
        description: "Create or update a credential handle. Secret values are never accepted as metaagent command arguments.",
    },
    MetaCommandDoc {
        name: "credential remove",
        aliases: &["credential remove"],
        usage: "credential remove <credential-id>",
        examples: &["credential remove credential-1"],
        tags: &["credential", "vault", "sensitive", "remove"],
        intents: &["remove credential handle", "delete credential metadata"],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "credential handle metadata only",
        routed: true,
        description: "Remove a credential handle without reading raw secret values.",
    },
    MetaCommandDoc {
        name: "credential vault",
        aliases: &["credential vault status", "credential vault manage"],
        usage: "credential vault <status|manage>",
        examples: &["credential vault status", "credential vault manage"],
        tags: &["credential", "vault", "sensitive"],
        intents: &["open vault management", "check vault status", "enter new secret in vault"],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "vault unlock and management request only",
        routed: true,
        description: "Request vault status or user-facing vault management. Raw secret values are not returned to the metaagent.",
    },
    MetaCommandDoc {
        name: "credential secret mutation",
        aliases: &["credential set", "credential set-secret", "credential delete-secret"],
        usage: "credential set|set-secret|delete-secret ...",
        examples: &[],
        tags: &["credential", "vault", "sensitive"],
        intents: &["set secret value", "delete secret value", "write raw secret"],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Deny,
        authority: "denied",
        routed: false,
        description: "Denied for metaagents. Secret values must be entered through user or worker credential interactions, not metaagent command text.",
    },
    MetaCommandDoc {
        name: "session new",
        aliases: &[
            "session create",
            "session attach",
            "session use",
            "session delete",
        ],
        usage: "session new|create|attach|use|delete ...",
        examples: &[],
        tags: &["session", "denied"],
        intents: &["create session", "attach session", "switch session", "delete session"],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Deny,
        authority: "denied",
        routed: false,
        description: "Denied for metaagents. Metaagents must operate inside their containing session.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaCommandExecutionPolicy {
    Routed,
    Denied { message: &'static str },
    NotRouted { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaCommandParseError {
    message: &'static str,
}

impl MetaCommandParseError {
    pub(crate) fn message(&self) -> &'static str {
        self.message
    }
}

pub(crate) fn tokenize_command(input: &str) -> Result<Vec<String>, MetaCommandParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    for ch in input.trim().chars() {
        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if escaping {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(MetaCommandParseError {
            message: "unterminated quote",
        });
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

pub(crate) fn search_commands(args: MetaCommandSearchArgs) -> Vec<serde_json::Value> {
    let query_terms = args.query.as_deref().map(search_terms);
    let tag = args.tag.map(|value| value.to_lowercase());
    let scope = args.scope.map(|value| value.to_lowercase());
    let policy = args.policy.map(|value| value.to_lowercase());
    let limit = args.limit.unwrap_or(50).clamp(1, 100);

    let mut commands: Vec<(&MetaCommandDoc, Option<CommandSearchMatch>)> = META_COMMANDS
        .iter()
        .filter(|command| {
            tag.as_ref().is_none_or(|tag| {
                command
                    .tags
                    .iter()
                    .any(|candidate| candidate.to_lowercase() == *tag)
            })
        })
        .filter(|command| scope.as_ref().is_none_or(|scope| command.scope == scope))
        .filter(|command| {
            args.mutates
                .is_none_or(|mutates| command.mutates == mutates)
        })
        .filter(|command| {
            policy
                .as_ref()
                .is_none_or(|policy| command.policy.as_str() == policy)
        })
        .filter_map(|command| match query_terms.as_ref() {
            Some(terms) => score_command(command, terms).map(|score| (command, Some(score))),
            None => Some((command, None)),
        })
        .collect();

    if query_terms.is_some() {
        commands.sort_by(|(left, left_match), (right, right_match)| {
            right_match
                .as_ref()
                .map_or(0, |search_match| search_match.score)
                .cmp(
                    &left_match
                        .as_ref()
                        .map_or(0, |search_match| search_match.score),
                )
                .then_with(|| right.routed.cmp(&left.routed))
                .then_with(|| left.policy.as_str().cmp(right.policy.as_str()))
                .then_with(|| left.name.cmp(right.name))
        });
    }

    commands
        .into_iter()
        .take(limit)
        .map(|(command, search_match)| {
            search_match.map_or_else(
                || command_json(command),
                |search_match| command_search_json(command, search_match),
            )
        })
        .collect()
}

pub(crate) fn list_commands(args: MetaCommandListArgs) -> Vec<serde_json::Value> {
    search_commands(MetaCommandSearchArgs {
        query: None,
        tag: args.tag,
        scope: args.scope,
        mutates: args.mutates,
        policy: args.policy,
        limit: args.limit,
    })
}

pub(crate) fn command_docs(args: MetaCommandDocsArgs) -> Option<serde_json::Value> {
    find_command(&args.command).map(command_json)
}

pub(crate) fn execution_policy(tokens: &[String]) -> MetaCommandExecutionPolicy {
    let Some(first) = tokens.first().map(|token| token.to_lowercase()) else {
        return MetaCommandExecutionPolicy::NotRouted {
            message: "empty metaagent command".to_string(),
        };
    };
    let normalized = if first == "session" {
        tokens
            .get(1)
            .map(|subcommand| format!("session {}", subcommand.to_lowercase()))
            .unwrap_or_else(|| "session".to_string())
    } else if matches!(first.as_str(), "agent" | "workflow") {
        tokens
            .get(1)
            .map(|subcommand| format!("{first} {}", subcommand.to_lowercase()))
            .unwrap_or_else(|| first.clone())
    } else {
        first.clone()
    };
    if let Some(family_policy) = routed_family_policy(&first, tokens) {
        return family_policy;
    }
    let Some(command) = find_command(&normalized).or_else(|| find_command(tokens[0].as_str()))
    else {
        return MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is not registered for arroba.meta.run_command",
                tokens[0]
            ),
        };
    };
    match command.policy {
        MetaCommandPolicy::Deny => MetaCommandExecutionPolicy::Denied {
            message: "metaagents cannot create, attach to, switch, or delete sessions",
        },
        MetaCommandPolicy::Approval if !command.routed => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` requires approval policy and is not routed for metaagent command execution yet",
                command.name
            ),
        },
        _ if command.routed => MetaCommandExecutionPolicy::Routed,
        _ => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is documented in the metaagent command registry but is not routed for execution yet",
                command.name
            ),
        },
    }
}

fn routed_family_policy(first: &str, tokens: &[String]) -> Option<MetaCommandExecutionPolicy> {
    match first {
        "agent" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "spawn" | "focus" | "alias" | "name" | "delete" | "destroy"
                | "remove",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `agent list`, `agent spawn`, `agent focus`, `agent alias`, and `agent delete` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "workflow" => match tokens.get(1).map(String::as_str) {
            None
            | Some(
                "list" | "ls" | "new" | "create" | "resolve" | "show" | "get" | "alias"
                | "name" | "run" | "start" | "runs" | "get-run" | "run-status" | "cancel"
                | "resume",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            Some("node")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some(
                        "add"
                            | "remove"
                            | "delete"
                            | "instructions"
                            | "instruct"
                            | "can-complete"
                            | "complete"
                            | "intermediate-output"
                            | "intermediate"
                            | "wait-for-all-inputs"
                            | "wait-all"
                            | "join"
                            | "max-turns"
                    )
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("endpoint")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some("new" | "create" | "alias" | "name")
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("edge")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some("add" | "remove" | "delete")
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            _ => {
                if let Some(documented) = documented_workflow_command(tokens) {
                    Some(policy_for_documented_command(documented))
                } else {
                    Some(MetaCommandExecutionPolicy::NotRouted {
                        message: "routed workflow commands: `workflow list`, `workflow new`, `workflow resolve`, `workflow alias`, `workflow node add/remove/instructions/can-complete/intermediate-output/wait-for-all-inputs/max-turns`, `workflow endpoint new/alias`, `workflow edge add/remove`, `workflow run`, `workflow runs`, `workflow get-run`, `workflow cancel`, and `workflow resume`".to_string(),
                    })
                }
            }
        },
        "mcp" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "show" | "get" | "install-json" | "update-json"
                | "uninstall" | "remove" | "import" | "grant" | "revoke",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `mcp list`, `mcp show`, `mcp install-json`, `mcp update-json`, `mcp uninstall`, `mcp import`, `mcp grant`, and `mcp revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "skill" | "skills" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "show" | "get" | "install" | "update" | "uninstall"
                | "remove" | "import" | "grant" | "revoke",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `skill list`, `skill show`, `skill install`, `skill update`, `skill uninstall`, `skill import`, `skill grant`, and `skill revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "extension" | "extensions" => match (
            tokens.get(1).map(String::as_str),
            tokens.get(2).map(String::as_str),
        ) {
            (Some("import"), Some("providers")) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `extension import providers` is routed for metaagent extension command execution yet".to_string(),
            }),
        },
        "slice" => Some(MetaCommandExecutionPolicy::Denied {
            message: "metaagents cannot manage slices or choose slice placement; delegate environment work to regular agents",
        }),
        "credential" | "credentials" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "get" | "show" | "upsert-json" | "remove") => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("vault") if matches!(tokens.get(2).map(String::as_str), Some("status" | "manage")) => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("set" | "set-secret" | "delete" | "delete-secret") => {
                Some(MetaCommandExecutionPolicy::Denied {
                    message: "metaagents cannot pass credential secret values through run_command; use worker credential interactions and resolve them explicitly",
                })
            }
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `credential list`, `credential get`, `credential upsert-json`, `credential remove`, `credential vault status`, and `credential vault manage` are routed for metaagent command execution".to_string(),
            }),
        },
        _ => None,
    }
}

fn documented_workflow_command(tokens: &[String]) -> Option<&'static MetaCommandDoc> {
    let first = tokens.first()?;
    if first != "workflow" {
        return None;
    }
    let candidates = [
        tokens
            .get(1)
            .map(|subcommand| format!("workflow {}", subcommand.to_lowercase())),
        tokens.get(2).map(|nested| {
            format!(
                "workflow {} {}",
                tokens.get(1).map(String::as_str).unwrap_or_default(),
                nested.to_lowercase()
            )
        }),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|candidate| find_exact_command(&candidate))
}

fn policy_for_documented_command(command: &'static MetaCommandDoc) -> MetaCommandExecutionPolicy {
    match command.policy {
        MetaCommandPolicy::Deny => MetaCommandExecutionPolicy::Denied {
            message: command.description,
        },
        _ if command.routed => MetaCommandExecutionPolicy::Routed,
        _ => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is documented in the metaagent command registry but is not routed for execution yet: {}",
                command.name, command.description
            ),
        },
    }
}

#[cfg(test)]
const ROUTED_COMMAND_CASES: &[&str] = &[
    "prompt agent-2 investigate",
    "agent list",
    "agent ls",
    "agent spawn reviewer",
    "agent focus reviewer",
    "agent alias reviewer code-reviewer",
    "agent name reviewer code-reviewer",
    "agent delete code-reviewer",
    "agent destroy code-reviewer",
    "agent remove code-reviewer",
    "workflow",
    "workflow list",
    "workflow ls",
    "workflow new qa-flow",
    "workflow create qa-flow",
    "workflow resolve qa-flow",
    "workflow show qa-flow",
    "workflow alias wf-1 qa-flow",
    "workflow node add qa-flow reviewer",
    "workflow node remove qa-flow node-1",
    "workflow node instructions qa-flow node-1 Review the implementation",
    "workflow node can-complete qa-flow node-1 true",
    "workflow node intermediate-output qa-flow node-1 false",
    "workflow node wait-for-all-inputs qa-flow node-1 true",
    "workflow node max-turns qa-flow node-1 3",
    "workflow endpoint new qa-flow node-1 default",
    "workflow endpoint create qa-flow node-1 default",
    "workflow endpoint alias qa-flow endpoint-1 default",
    "workflow edge add qa-flow node-1 node-2",
    "workflow edge remove qa-flow edge-1",
    "workflow run qa-flow default Run QA",
    "workflow start qa-flow default Run QA",
    "workflow runs qa-flow",
    "workflow get-run run-1",
    "workflow run get run-1",
    "workflow cancel run-1",
    "workflow resume run-1",
    "mcp list",
    "mcp ls",
    "mcp show playwright",
    "mcp get playwright",
    "mcp install-json {\"name\":\"playwright\"}",
    "mcp update-json {\"name\":\"playwright\"}",
    "mcp uninstall playwright",
    "mcp remove playwright",
    "mcp import codex playwright",
    "mcp grant reviewer playwright",
    "mcp revoke reviewer playwright",
    "skill list",
    "skill ls",
    "skill show browser-qa",
    "skill get browser-qa",
    "skill install ./skills/browser-qa",
    "skill update ./skills/browser-qa",
    "skill uninstall browser-qa",
    "skill remove browser-qa",
    "skill import codex browser-qa",
    "skill grant reviewer browser-qa",
    "skill revoke reviewer browser-qa",
    "skills list",
    "skills grant reviewer browser-qa",
    "credential list",
    "credential ls",
    "credential get credential-1",
    "credential show credential-1",
    "credential upsert-json {\"id\":\"credential-1\"}",
    "credential remove credential-1",
    "credential vault status",
    "credential vault manage",
    "credentials list",
    "credentials get credential-1",
];

fn find_command(command: &str) -> Option<&'static MetaCommandDoc> {
    let normalized = command.to_lowercase();
    META_COMMANDS
        .iter()
        .filter_map(|candidate| {
            command_match_len(candidate, &normalized).map(|len| (len, candidate))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, command)| command)
}

fn find_exact_command(command: &str) -> Option<&'static MetaCommandDoc> {
    let normalized = command.to_lowercase();
    META_COMMANDS.iter().find(|candidate| {
        candidate.name == normalized || candidate.aliases.iter().any(|alias| *alias == normalized)
    })
}

fn command_match_len(command: &MetaCommandDoc, normalized: &str) -> Option<usize> {
    let mut matches = Vec::new();
    if token_prefix_matches(normalized, command.name) {
        matches.push(command.name.len());
    }
    matches.extend(command.aliases.iter().filter_map(|alias| {
        let alias = alias.to_lowercase();
        token_prefix_matches(normalized, &alias).then_some(alias.len())
    }));
    matches.into_iter().max()
}

fn token_prefix_matches(normalized: &str, candidate: &str) -> bool {
    normalized == candidate
        || normalized
            .strip_prefix(candidate)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

#[derive(Debug, Clone)]
struct CommandSearchMatch {
    score: i64,
    matched_fields: Vec<&'static str>,
    matched_terms: Vec<String>,
}

#[derive(Debug, Clone)]
struct SearchTerm {
    original: String,
    expanded: Vec<String>,
}

fn search_terms(query: &str) -> Vec<SearchTerm> {
    normalize_tokens(query)
        .into_iter()
        .map(|term| {
            let mut expanded = vec![term.clone()];
            for synonym in synonyms_for(&term) {
                if !expanded.iter().any(|candidate| candidate == synonym) {
                    expanded.push((*synonym).to_string());
                }
            }
            SearchTerm {
                original: term,
                expanded,
            }
        })
        .collect()
}

fn score_command(command: &MetaCommandDoc, terms: &[SearchTerm]) -> Option<CommandSearchMatch> {
    if terms.is_empty() {
        return Some(CommandSearchMatch {
            score: 1,
            matched_fields: Vec::new(),
            matched_terms: Vec::new(),
        });
    }

    let fields: [(&'static str, i64, String); 8] = [
        ("name", 90, command.name.to_string()),
        ("aliases", 75, command.aliases.join(" ")),
        ("intents", 55, command.intents.join(" ")),
        ("description", 35, command.description.to_string()),
        ("usage", 30, command.usage.to_string()),
        ("examples", 20, command.examples.join(" ")),
        ("tags", 20, command.tags.join(" ")),
        ("authority", 10, command.authority.to_string()),
    ];
    let field_tokens: Vec<(&'static str, i64, Vec<String>)> = fields
        .iter()
        .map(|(name, weight, text)| (*name, *weight, normalize_tokens(text)))
        .collect();
    let mut score = 0;
    let mut matched_fields = Vec::new();
    let mut matched_terms = Vec::new();

    for term in terms {
        let mut term_matched = false;
        for (field_name, weight, tokens) in &field_tokens {
            let field_hits = term
                .expanded
                .iter()
                .filter(|expanded| tokens.iter().any(|token| token == *expanded))
                .count();
            if field_hits > 0 {
                score += *weight * i64::try_from(field_hits).unwrap_or(1);
                term_matched = true;
                if !matched_fields.contains(field_name) {
                    matched_fields.push(*field_name);
                }
            }
        }
        if term_matched
            && !matched_terms
                .iter()
                .any(|matched| matched == &term.original)
        {
            matched_terms.push(term.original.clone());
        }
    }

    if matched_terms.len() == terms.len() {
        score += 40;
    }
    if terms.iter().any(|term| {
        term.expanded.iter().any(|expanded| {
            command.name == expanded || command.aliases.contains(&expanded.as_str())
        })
    }) {
        score += 50;
    }
    if command.routed {
        score += 5;
    }

    (score > 0).then_some(CommandSearchMatch {
        score,
        matched_fields,
        matched_terms,
    })
}

fn normalize_tokens(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|raw| {
            let token = normalize_token(raw);
            (!token.is_empty()).then_some(token)
        })
        .collect()
}

fn normalize_token(raw: &str) -> String {
    if matches!(
        raw,
        "a" | "an"
            | "and"
            | "as"
            | "do"
            | "for"
            | "how"
            | "i"
            | "in"
            | "it"
            | "me"
            | "of"
            | "on"
            | "please"
            | "the"
            | "to"
            | "with"
    ) {
        return String::new();
    }
    if raw.len() > 4 && raw.ends_with('s') {
        raw.trim_end_matches('s').to_string()
    } else {
        raw.to_string()
    }
}

fn synonyms_for(term: &str) -> &'static [&'static str] {
    match term {
        "abort" => &["cancel", "stop"],
        "add" => &["create", "install", "new"],
        "ask" => &["prompt", "delegate"],
        "capability" => &["mcp", "skill", "tool"],
        "continue" => &["resume"],
        "create" => &["spawn", "new", "add", "make"],
        "credential" => &["secret", "vault"],
        "delegate" => &["prompt", "agent", "worker"],
        "execute" => &["run", "start"],
        "give" => &["grant"],
        "make" => &["create", "spawn", "new", "add"],
        "pause" => &["cancel", "stop"],
        "remove" => &["delete", "uninstall"],
        "run" => &["start", "execute"],
        "secret" => &["credential", "vault"],
        "see" => &["list", "inspect"],
        "start" => &["run", "spawn", "create"],
        "stop" => &["cancel", "abort"],
        "store" => &["credential", "vault", "upsert"],
        "task" => &["prompt", "delegate", "workflow"],
        "tell" => &["prompt", "delegate"],
        "tool" => &["mcp", "skill", "capability"],
        "worker" => &["agent"],
        _ => &[],
    }
}

fn command_json(command: &MetaCommandDoc) -> serde_json::Value {
    serde_json::json!({
        "name": command.name,
        "aliases": command.aliases,
        "usage": command.usage,
        "examples": command.examples,
        "tags": command.tags,
        "intents": command.intents,
        "scope": command.scope,
        "mutates": command.mutates,
        "metaagent_policy": command.policy.as_str(),
        "authority": command.authority,
        "routed": command.routed,
        "description": command.description,
    })
}

fn command_search_json(
    command: &MetaCommandDoc,
    search_match: CommandSearchMatch,
) -> serde_json::Value {
    let mut value = command_json(command);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "search_score".to_string(),
            serde_json::json!(search_match.score),
        );
        object.insert(
            "matched_fields".to_string(),
            serde_json::json!(search_match.matched_fields),
        );
        object.insert(
            "matched_terms".to_string(),
            serde_json::json!(search_match.matched_terms),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_routed_examples_match_execution_policy() {
        for command in META_COMMANDS.iter().filter(|command| command.routed) {
            for example in command
                .examples
                .iter()
                .filter(|example| !example.starts_with("arroba.meta."))
            {
                let tokens = tokenize_command(example).unwrap_or_else(|error| {
                    panic!("documented example `{example}` should parse: {error:?}")
                });
                assert_eq!(
                    execution_policy(&tokens),
                    MetaCommandExecutionPolicy::Routed,
                    "documented example `{example}` for `{}` is not routed",
                    command.name
                );
            }
        }
    }

    #[test]
    fn routed_command_cases_match_execution_policy() {
        for command in ROUTED_COMMAND_CASES {
            let tokens = tokenize_command(command).unwrap_or_else(|error| {
                panic!("routed command case `{command}` should parse: {error:?}")
            });
            assert_eq!(
                execution_policy(&tokens),
                MetaCommandExecutionPolicy::Routed,
                "`{command}` should be routed by the metaagent command registry",
            );
        }
    }

    #[test]
    fn list_commands_filters_descriptor_table_without_query() {
        let commands = list_commands(MetaCommandListArgs {
            tag: Some("credential".to_string()),
            scope: Some("global".to_string()),
            mutates: Some(true),
            policy: Some("allow".to_string()),
            limit: Some(10),
        });

        assert!(commands.iter().any(|command| {
            matches!(
                command.get("name").and_then(serde_json::Value::as_str),
                Some("credential upsert-json" | "credential remove" | "credential vault")
            )
        }));
        assert!(commands.iter().all(|command| {
            command
                .get("tags")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("credential")))
                && command.get("scope").and_then(serde_json::Value::as_str) == Some("global")
                && command.get("mutates").and_then(serde_json::Value::as_bool) == Some(true)
                && command
                    .get("metaagent_policy")
                    .and_then(serde_json::Value::as_str)
                    == Some("allow")
        }));
    }

    #[test]
    fn search_commands_ranks_natural_language_intents() {
        let cases = [
            ("create new agent", "agent spawn"),
            ("make worker", "agent spawn"),
            ("delegate task", "prompt"),
            ("store credential", "credential upsert-json"),
            ("enter new secret", "credential vault"),
            ("stop workflow", "workflow cancel"),
            ("continue workflow", "workflow resume"),
            ("connect workflow nodes", "workflow edge add"),
        ];

        for (query, expected_name) in cases {
            let commands = search_commands(MetaCommandSearchArgs {
                query: Some(query.to_string()),
                tag: None,
                scope: None,
                mutates: None,
                policy: Some("allow".to_string()),
                limit: Some(3),
            });
            assert!(
                commands.iter().any(|command| {
                    command.get("name").and_then(serde_json::Value::as_str) == Some(expected_name)
                }),
                "`{query}` should return `{expected_name}` near the top: {commands:?}"
            );
            assert!(
                commands
                    .iter()
                    .all(|command| command.get("search_score").is_some()),
                "query searches should include match diagnostics"
            );
        }
    }

    #[test]
    fn search_commands_finds_worker_capability_grants() {
        let commands = search_commands(MetaCommandSearchArgs {
            query: Some("give worker tool".to_string()),
            tag: None,
            scope: Some("session".to_string()),
            mutates: Some(true),
            policy: Some("allow".to_string()),
            limit: Some(4),
        });

        assert!(
            commands.iter().any(|command| {
                matches!(
                    command.get("name").and_then(serde_json::Value::as_str),
                    Some("mcp grant" | "skill grant")
                )
            }),
            "grant command should be discoverable: {commands:?}"
        );
    }

    #[test]
    fn search_commands_finds_workflow_guides() {
        let commands = search_commands(MetaCommandSearchArgs {
            query: Some("build workflow tutorial connect nodes run endpoint".to_string()),
            tag: Some("guide".to_string()),
            scope: Some("session".to_string()),
            mutates: Some(false),
            policy: Some("allow".to_string()),
            limit: Some(5),
        });

        assert!(
            commands.iter().any(|command| {
                command.get("name").and_then(serde_json::Value::as_str)
                    == Some("workflow tutorial basic")
            }),
            "workflow tutorial should be discoverable: {commands:?}"
        );
        let docs = command_docs(MetaCommandDocsArgs {
            command: "agent app guide".to_string(),
        })
        .expect("agent app guide docs should exist");
        assert_eq!(
            docs.get("routed").and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn workflow_command_catalog_includes_unrouted_workflow_capabilities() {
        let required = [
            "workflow node intermediate-output-schema",
            "workflow endpoint bind",
            "workflow endpoint remove",
            "workflow run-output-schema",
            "workflow max-turns",
            "workflow node extensions",
            "workflow pane",
        ];

        for command in required {
            let docs = command_docs(MetaCommandDocsArgs {
                command: command.to_string(),
            })
            .unwrap_or_else(|| panic!("missing docs for `{command}`"));
            assert_eq!(
                docs.get("routed").and_then(serde_json::Value::as_bool),
                Some(false),
                "`{command}` should be documented but not routed"
            );
        }
    }

    #[test]
    fn documented_unrouted_workflow_commands_return_specific_policy_errors() {
        let cases = [
            "workflow endpoint bind qa-flow default queue-1",
            "workflow run-output-schema qa-flow schema-1",
            "workflow edit qa-flow",
        ];

        for command in cases {
            let tokens = tokenize_command(command).expect("command should tokenize");
            let documented = documented_workflow_command(&tokens).unwrap_or_else(|| {
                panic!("`{command}` should resolve to documented workflow docs")
            });
            assert!(
                !documented.routed,
                "`{command}` should resolve to unrouted docs `{}`",
                documented.name
            );
            let policy = execution_policy(&tokens);
            match policy {
                MetaCommandExecutionPolicy::NotRouted { message } => {
                    assert!(
                        message.contains("not routed")
                            || message.contains("not available")
                            || message.contains("not currently exposed")
                            || message.contains("not implemented"),
                        "`{command}` should produce a specific documented error: {message}"
                    );
                }
                MetaCommandExecutionPolicy::Denied { message } => {
                    assert!(
                        message.contains("not routed")
                            || message.contains("not available")
                            || message.contains("not currently exposed")
                            || message.contains("not implemented"),
                        "`{command}` should produce a specific documented error: {message}"
                    );
                }
                MetaCommandExecutionPolicy::Routed => {
                    panic!("`{command}` should not be routed");
                }
            }
        }
    }

    #[test]
    fn tokenizer_preserves_quoted_prompt_text() {
        assert_eq!(
            tokenize_command(r#"prompt worker "please inspect the failing test""#)
                .expect("double-quoted prompt should parse"),
            vec!["prompt", "worker", "please inspect the failing test"],
        );
        assert_eq!(
            tokenize_command(r#"prompt worker 'do not expand \slashes'"#)
                .expect("single-quoted prompt should parse"),
            vec!["prompt", "worker", r#"do not expand \slashes"#],
        );
        assert_eq!(
            tokenize_command(r#"prompt worker escaped\ space"#)
                .expect("escaped whitespace should parse"),
            vec!["prompt", "worker", "escaped space"],
        );
    }

    #[test]
    fn tokenizer_rejects_unterminated_quotes() {
        let error = tokenize_command(r#"prompt worker "unterminated"#)
            .expect_err("unterminated quotes should fail");
        assert_eq!(error.message(), "unterminated quote");
    }

    #[test]
    fn command_docs_do_not_advertise_unrouted_subcommands() {
        let forbidden = [
            ("prompt", &["--wait", "--show-reply", "--show-summary"][..]),
            ("workflow", &["edit"][..]),
            ("workflow node add", &["node-name", "node alias"][..]),
            ("mcp", &["test", "adapter", "connector"][..]),
            ("skill", &["grants", "script", "connector"][..]),
            ("slice", &["reset-state"][..]),
            ("credential", &["set-secret", "delete-secret"][..]),
        ];

        for (command, terms) in forbidden {
            let docs = command_docs(MetaCommandDocsArgs {
                command: command.to_string(),
            })
            .unwrap_or_else(|| panic!("missing docs for `{command}`"));
            let rendered = serde_json::json!({
                "usage": docs.get("usage"),
                "aliases": docs.get("aliases"),
                "examples": docs.get("examples"),
            })
            .to_string();
            for term in terms {
                assert!(
                    !rendered.contains(term),
                    "`{command}` docs should not advertise unrouted `{term}`: {rendered}"
                );
            }
        }
    }

    #[test]
    fn command_docs_prefer_specific_denial_policy_entries() {
        let docs = command_docs(MetaCommandDocsArgs {
            command: "credential set-secret".to_string(),
        })
        .expect("credential secret mutation docs should exist");

        assert_eq!(
            docs.get("name").and_then(serde_json::Value::as_str),
            Some("credential secret mutation")
        );
        assert_eq!(
            docs.get("metaagent_policy")
                .and_then(serde_json::Value::as_str),
            Some("deny")
        );
        assert_eq!(
            docs.get("routed").and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }
}
