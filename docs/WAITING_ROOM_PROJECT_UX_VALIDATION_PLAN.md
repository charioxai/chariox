# MP-02 / MP-08 waiting-room Project UX acceptance

Owner reports from 2026-10-16 define these required `apps-tui-ux` scenarios.
Run on the exact paired OSS/Cloud heads through an owned hosted WSS stack,
using real browser clicks and keys at 1440×900, DPR 1 and DPR 2. Capture video,
screenshots and console errors, then review the recordings for stuck or blank
frames. Exercise the real TUI for the shared workspace and Project actions.
Source tests supplement these drills. Record the first failure on the base UI
and the same steps passing on the candidate; a source-only pass closes no row.

| ID | Required user flow and assertion |
| --- | --- |
| OWNER-13 | Select machine/kernel and an ordinary non-git workspace. The worktree row immediately says “Worktrees not available”; it never offers a worktree choice or shows a subsequent loading message. Check Git repositories and repositories without commits separately. Directory-search choices carry availability with the path; no client infers Git from a name. |
| OWNER-14 | Non-git workspace paths have no parenthesis. A Git workspace with an origin remote displays its own `owner/repo`. A repository without a remote has no inferred label. Repeat after switching workspaces and kernels. |
| OWNER-15 | Pin/unpin from project actions (P in TUI). Pins form the first section and persist across reload/kernel restart. The remaining projects use the exact activity time displayed on their rows. A session change updates both order and age live. |
| OWNER-19 | The project selector says “Auto (resolved project name)”. Left/right and the popup browse all active projects of the selected kernel across workspaces. Name New in the waiting room, create a real session and verify its kernel-owned project name and membership. Duplicate/invalid names fail without creating a project. Exercise one short real provider turn on the validation model and verify its resolved ID. |
| OWNER-20 | The discontinued synchronization feature has no controls, keyboard targets, badges, popup entries or help in Web/TUI/Settings/command completion. Its implementation and engineering history remain in source. |
| OWNER-21 | Existing waiting-room session rows retain their open action and have no adjacent pencil button. Session lifecycle menus still work. |

For each row record exact heads and binary SHA-256, commands, exit codes,
DPR, hosted origin/relay, resources, evidence and cleanup. Keep acceptance rows
NOT RUN or FAIL until real drills pass; a missing real resource requires the
specific coordinator/owner action. Never use direct IPC alone as acceptance.
