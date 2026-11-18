import type { AppError } from './index.js';
type ImportPromise<T> = Promise<T>;
export const workspace: {
  /** JSON text: `{files:[{path,status,additions,deletions}],branch,defaultBranch,remote}`. */
  changes(worktree: string): ImportPromise<string>;
  /** JSON text: `{commit,branch,createdBranch}`. */
  commit(worktree: string, files: string[], message: string, branch: string | null): ImportPromise<string>;
  /** JSON text: `{commit,branch}`. */
  push(worktree: string, account: string): ImportPromise<string>;
};
export type { AppError };
