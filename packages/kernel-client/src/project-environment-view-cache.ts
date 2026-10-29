// MP-08 / MP-10 / MP-11: bounded render memory only; every open still refreshes from the kernel.
import type { ProjectEnvironment } from "./kernel-types-project-environment-aggregate.js"

export function createEnvironmentViewCache() {
  let bound: unknown
  const snapshots = new Map<string, ProjectEnvironment>()
  function bind(scope: unknown) {
    if (scope !== bound) { snapshots.clear(); bound = scope }
  }
  return {
    peek(scope: unknown, projectId: string): ProjectEnvironment | null {
      bind(scope)
      return snapshots.get(projectId) ?? null
    },
    remember(scope: unknown, snapshot: ProjectEnvironment) {
      bind(scope)
      snapshots.delete(snapshot.local_project_id)
      snapshots.set(snapshot.local_project_id, snapshot)
      if (snapshots.size > 8) snapshots.delete(snapshots.keys().next().value!)
    },
    forget(scope: unknown, projectId: string) { bind(scope); snapshots.delete(projectId) },
  }
}
