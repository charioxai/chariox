// MP-08 / MP-09 / MP-10 / MP-11 A12: compact home task projection, no authority policy.
import type { AgentTaskExecution, RuntimeSession } from "./kernel-types-session.js"

export function withAgentTaskUpdate(session: RuntimeSession, sessionId: string, task: AgentTaskExecution): RuntimeSession {
  if (session.id !== sessionId || task.room_id !== sessionId || typeof task.task_id !== "string" || !task.task_id
    || !Number.isSafeInteger(task.revision) || task.revision < 1
    || !session.agents.some(agent => agent.id === task.agent_id)) return session
  const tasks = session.agent_tasks ?? []
  const previous = tasks.find(t => t.task_id === task.task_id)
  if (previous && previous.revision >= task.revision) return session
  return { ...session, agent_tasks: previous
    ? tasks.map(t => t.task_id === task.task_id ? task : t) : [...tasks, task] }
}
