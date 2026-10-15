// MP-08 / MP-10 / MP-11: Project identity is independent of cwd and folders.
import type { RuntimeSession } from "./cli-types.js";
import { projectSelectionFromId } from "./waiting-room-projects.js";
import type { SkillsContext } from "@chariox/kernel-client/skills-client";
export function tuiSkillsContext(session: RuntimeSession, agentId: string | null, attached: boolean, projectSelectionId = "default", disconnected = false): SkillsContext {
    const selection = projectSelectionFromId(projectSelectionId);
    const projectId = attached ? session.project_id : selection.kind === "existing" ? selection.project_id : null;
    const agent = attached ? session.agents.find(a => a.id === agentId) : null;
    return { key: JSON.stringify([attached, session.id, projectId, agent?.id, agent?.provider, agent?.account_profile, agent?.remote_execution, disconnected]), projectId: projectId || null, sessionId: attached ? session.id : null, agentId: agent?.id ?? null };
}
