# MP-08 / MP-09 / MP-10 / MP-11 — selected environment and providers on a rented machine

Drill for the S4 slice. It runs on the ordinary managed VM; there is no job tool, no SSH and no remote agent
except the one leased turn. Compute vendors, images, quotas and prices stay in Cloud.

1. Build the base, enroll a real home kernel in an isolated hosted Cloud stack as its owner, link the Codex,
   OpenCode and Claude profiles, sign in `gh` in the home kernel's own HOME, and create a small Project whose
   `origin` is the private GitHub repository.
2. Launch two machines with the ordinary managed-environment request the Waiting Room sends:
   - Empty kernel context and Empty development, selected account Codex only.
   - Copied kernel context (portable configuration and the target-sealed Vault), Current Project, selected
     accounts Codex and OpenCode, selected GitHub credential.
   The client waits for `awaiting_context`, takes the Cloud ticket and starts the source transfer. The source
   waits for the new machine's relay socket before arming (it confirms with Cloud a moment earlier).
3. Through the hosted relay, as the owner, open a Room on the target kernel and check: the target has its own
   kernel identity, the Project is imported at the reported path with the same head, `gh` is signed in, and
   `git ls-remote origin HEAD` / `git fetch --depth 1 origin HEAD` reach the private repository.
4. Run one short real turn per selected provider with the validation models (Codex `gpt-6-luna` low, Claude
   `claude-haiku-5-5`, OpenCode its cheapest permitted model). Confirm by a joined-token reply, not a word that
   is also in the prompt.
5. Run one leased turn: a home-Room agent with `kernel_ref` set to the machine. The target logs
   `launching remote lease provider run`; the transcript stays in the home Room.
6. Scan the target for credentials by path/boolean only. Send SHA-256 digests of the values that must not be there
   (infrastructure key, signing key, Vault passphrases, unselected account logins), never the values. Selected
   logins may appear only inside their provider profile or the GitHub CLI config. A redeemed bootstrap grant must
   not keep its token: root blanks it in `/etc/chariox/bootstrap/managed-bootstrap.json` once Cloud confirmed the
   kernel (no `mkboot_` value, `token` empty, `redeemedGrantBindingDigest` present).
7. Release every machine through the Cloud lifecycle, read provider usage before delete, and census twice at
   least 60 seconds apart.

Launch through the Waiting Room instead of the Cloud API: select the home Machine, Kernel and Project, pick the
Provider, Account and Model, choose "+ New Chariox-managed machine...", set Kernel context from (the home kernel),
Development setup (Current Project), Provider accounts source (one account of the selected provider) and Git credentials
source (GitHub), press Done, and "Create machine and start session". The source kernel then asks "Review your Project
setup" in the Project's home session; the launch continues only after "Looks good, continue".

Claude on the machine: a Claude setup-token login is a Vault credential and is never transferred. Enroll it on the
machine's own Vault through the product input (`SetProviderAccountCredential` with the setup token, answering the
Vault interaction), then run the `claude-haiku-5-5` turn.

Known limits found by this drill: a Claude setup-token login is not transferable, so a transferred Claude account needs a
portable claude.ai file login; the Git credential import needs `gh` in the base image; the redeemed bootstrap grant stays
in the protected envelope file (the unit runs as `chariox` and cannot remove it); Project environment discovery borrows an
idle agent of the Project, so an idle agent whose provider login is expired stalls the launch for the utility timeout; the
launching web client did not reach a freshly ready machine's kernel within its two-minute window although the owner relay
client did.
