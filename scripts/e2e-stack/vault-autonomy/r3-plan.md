# Gmail signup round — MP-08 / MP-09 / MP-10 / MP-11

This plan is prepared, not executed. Do not open or modify the retained Gmail
profile before Miguel completes the sign-in described in
`agents/gmailprep/MORNING_GMAIL.md` in the lane state directory.

## Start on the retained kernel

Use `https://pr0-gmailprep.val.51-255-87-178.sslip.io/terminal?ui=os`, the drill
owner, Room `gmail_drill` (`18e45b6022ad6c00`), kernel
`kernel-6486dbcf110551f2` / **Vault-Gmail-Drill** on **b3-vault-drill**. A Room
on another kernel does not share its browser. Do not copy cookies or profiles.

Create a regular official Codex agent (`gpt-6-luna`, low) there and focus it in
the owner UI. Ask it to call `chariox.load_kernel_browser`; use ordinary browser
resource admission. Existing Gmail-tab access requires live owner focus. If a
parent delegates, share only the explicit required browser resource with its
direct local child using `chariox.kernel_browser_share`. Serialize mailbox
access. Do not restart, replace or clean this intentionally retained kernel.

First prove the inbox is signed in by observing the actual Gmail page. A file
or service status is insufficient. If Google challenges again, stop only
mail-dependent steps and record that Miguel must finish that challenge. Google
sign-in remains N/A-AUTOMATED; do not automate it.

Record exact kernel binary/source, Cloud bundle, provider run/model, real hosted
WSS connection and client shaping. Repeat service journeys at DPR 1 and 2;
retain screenshots and console/runtime/native-provider absence scans outside
the repository. Use a different disposable account/project per journey. Keep
cleanup handles in Vault and a nonsecret resource manifest before creating the
next resource; do not put credentials or confirmation URLs in evidence.

## Agent prompts and success gates

Send each prompt through the real Room prompt bar after replacing `RUN` with a
unique nonsecret run marker. Read the complete current AU row before starting.
The agent uses observed controls, generated Vault passwords bound to each
service origin, and the signed-in drill Gmail for confirmation messages.
Explicitly authorize only free service tiers and the listed disposable
resources. A payment/card requirement blocks only that service. Do not use
Google social sign-in, Miguel's personal service accounts or CAPTCHA bypasses.

**AU-06 — database signup and exact-file use**

> MP-08 / MP-09 / MP-10 / MP-11 AU-06 RUN: create your own disposable free Neon
> account using email signup and this drill Gmail for confirmation. Inspect the
> current signup page; use Supabase instead only if Neon's normal signup is
> unavailable and report that choice. Never automate a Google login or a bot
> challenge. Create one RUN database project. Capture its real connection secret
> directly into Vault, bound to the exact app workspace file required by the
> current API. Use the supported Vault file injection to configure the real
> app, without printing the connection string. Establish a missing/wrong
> configuration failure, then a successful SQL write and read through that app.
> Check that a different file cannot use this entry. Record resource IDs,
> nonsecret query proof and exact cleanup results. Delete the project and your
> disposable service account after verification. A hosted-app claim also needs
> an authorized real deployment target; report that seam separately if absent.

Use `capture_command` (for example, a normally authenticated Neon CLI command
whose stdout is the connection string) or `capture_file`, with its exact file
binding, followed by `vault.write_file`. Current browser capture creates a
Site-bound Browser credential; it cannot be relabeled as an Env/File entry.
If only that capture channel is available, report the missing channel instead
of reading/reposting the value or assuming that browser capture supports files.

References: [Neon accounts](https://neon.com/docs/manage/accounts),
[Neon project API](https://api-docs.neon.tech/reference/createproject),
[Supabase project/app tutorial](https://supabase.com/docs/guides/getting-started/tutorials/with-nextjs).
Database signup/query alone does not close the deployed-app part of AU-06.

**AU-10 — TOTP account, enrollment and two-period login**

> MP-08 / MP-09 / MP-10 / MP-11 AU-10 RUN: create your own disposable PyPI
> account through its ordinary email registration, confirmed using the drill
> Gmail. Do not publish a package. If registration requires human verification,
> record the challenge and stop that service. In account security, enroll an
> authenticator using the observed manual setup key. Capture that key straight
> to Vault with the TOTP service binding supported by this kernel; never copy
> it into chat or logs. Generate/fill codes through the kernel TOTP path, prove
> actual login, sign out, wait for a different real TOTP period with one timer
> wake, and prove another actual login. Keep recovery material in Vault until
> deletion is verified. Disable/delete only this disposable account and verify
> cleanup. Report enrollment, first login, second-period login and cleanup
> independently.

Reference: [PyPI TOTP enrollment](https://docs.pypi.org/your-account/two-factor-authentication/).
Inspect the real page for a manual key before relying on this candidate. Do
not substitute a generated local TOTP fixture for a service login.
At `68c0f04848`, browser capture always stores `injection: Browser`, while
`docs/VAULT_TOTP.md` provisions a TOTP seed through the protected owner secret
interaction. There is no browser-capture `totp` parameter. Preflight this seam
on the then-current kernel before enrollment. If unchanged, agent signup can
run, but fully autonomous manual-key capture remains blocked on that product
integration; never report ordinary Browser injection as a computed TOTP code.

**AU-09 and AU-11 — sender signup, key lifecycle and delivery**

> MP-08 / MP-09 / MP-10 / MP-11 AU-09/AU-11 RUN: create your own disposable
> Resend account through normal email signup, confirmed using the drill Gmail.
> Capture a newly created restricted sending API key directly into Vault with
> one exact command-environment use at the service's actual API origin. Prove
> absent-key refusal, authorized use, inventory and rotation with the binding
> preserved. Revoke the individual old key and prove the real API refuses it.
> For AU-09, add only a coordinator-authorized disposable sending domain or
> subdomain; publish the exact requested DKIM/SPF records through its authorized
> DNS path, wait via a real timer for public resolution and provider verified
> status, send one uniquely marked message to the drill Gmail, and observe its
> actual inbox delivery. Delete the created key/domain/DNS records/account and
> retain nonsecret cleanup receipts. Do not change Miguel's Cloudflare/Vercel
> projects. If a sending domain is unavailable, finish the key-lifecycle subset
> and report AU-09 domain verification/delivery BLOCKED with the exact missing
> domain authority. Provider test-event simulation is not real inbox delivery.

Reference: [Resend domain ownership and verification](https://resend.com/docs/dashboard/domains/introduction).
A free shared dynv6 hostname is not assumed to satisfy Resend's owned-domain
requirement. Gmail sign-in alone does not remove this DNS/domain dependency.

## Settle every journey

Persist resource IDs, credential handles, exact steps completed and cleanup
status before moving on. On a partial failure, delete the resources already
created; retain private recovery access until deletion is confirmed. Report
the first failing seam, never just a success/failure phrase from the model.
Record every row with plain-string dimension values in `ACCEPTANCE_UPDATE.json`.
Keep partial AU-06 and AU-09 results partial. AU-03/08 Stripe, Miguel's
Vercel/Cloudflare, AU-07 OAuth, AU-12 complete chain, AU-13 VM, AU-14 WBA,
AU-15 auth.md and AU-16 routing are not unlocked merely by Gmail sign-in.

End by stopping only newly created agents/processes and deleting this round's
disposable service resources. Preserve the Gmail Room, kernel, signed-in
profile, protected stack and shared provider accounts for subsequent drills.
