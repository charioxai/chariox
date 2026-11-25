# Landing 14 union review input (oss)
Scope (Miguel's decision): only what the union itself adds.
- merge-resolutions.diff: `git show --remerge-diff` of every union merge commit, i.e. how each merge conflict was resolved (auto-merge with conflict markers -> the union's code).
- union-only-commits.patch: every union commit that no PR merge brought in (compile fixes, protocol 1015/452 commit, test-drift fixes, gate fixes, budget trim, sweep fix, Ask-me fix, drill fix; Cloud: re-pins, bundle lazy-loading, c#763 revert), as patches.
Base of this PR = 'reviewed-stack': main + each PR at the head the union merged (plain git merge; conflicted files take the union's resolution). The companion PR (reviewed-stack -> union head) shows the resulting code.
Known gap: the late-window PRs were rebased onto union heads, so their history contains union commits; those are listed in union-only-commits.patch so nothing union-made is hidden.
