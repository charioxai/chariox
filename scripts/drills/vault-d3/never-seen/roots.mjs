// Scan roots per kernel (HARNESS.md section 1). Layout is frozen after the first dry run on each kernel type; scan.json prints the roots used.
import { existsSync } from "node:fs"
import { join } from "node:path"

/** home: the kernel's CHARIOX_HOME; extra: paths the driver wrote (room export, client console/HAR, shell history, job output). */
export function kernelRoots({ kernel, home, claudeHome, codexHome, workspace, extra = [] }) {
  const roots = [
    { target: "T1", kernel, kind: "dir", path: join(home, "state", "history") },
    { target: "T2", kernel, kind: "dir", path: join(claudeHome ?? "", "projects") },
    { target: "T2", kernel, kind: "dir", path: join(codexHome ?? "", "sessions") },
    { target: "T3", kernel, kind: "dir", path: join(home, "logs") },
    { target: "T4", kernel, kind: "dir", path: join(home, "state", "traces") },
    { target: "T9", kernel, kind: "dir", path: workspace ?? "" },
    { target: "T9", kernel, kind: "proc" },
    ...extra.map(e => ({ kernel, kind: "dir", ...e })),
  ]
  // Roots that do not exist on this kernel type are dropped here, so "0 files" in a root that does exist stays INVALID.
  return roots.filter(r => r.kind === "proc" || (r.path && existsSync(r.path)))
}
