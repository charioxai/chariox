# FA regression fixtures (MP-08 / MP-09 / MP-10 / MP-11)

Supplementary regression services for FA01-FA10 (never acceptance, see
`docs/AGENT_MODEL_OVERHAUL_PLAN.md`, "variant A"). Project `amunblock-fa`,
loopback only, base port `FA_PORT_BASE` (default 18100):

| Port | Service |
| --- | --- |
| +0 / +1 | Mailpit SMTP / web UI + API (`POST /api/v1/send`) |
| +2 | dex local OIDC, client `fa-app`, users `alice@fa.invalid` and `bob@fa.invalid` |
| +3 | Postgres 16 database `app` |
| +4 | stripe-mock (a Stripe API mock, **not** Stripe test mode) |

The host needs only `docker` (Compose v2) and `node`. `./up.sh` (dex's bcrypt hashes come from
the stack's own Postgres, `pgcrypto`), `node verify.mjs <receipt.json>` (needs the secrets file), `./status.sh`,
`./down.sh` (removes only this project and its generated state). Passwords and the OIDC client
secret are generated into `$FA_STATE_DIR/secrets.env` (mode 0600, default
`~/.chariox/dev/amunblock/fa`).

dex runs on the bridge network with its port published as `127.0.0.1:+2`, so it works on Docker
Desktop too. It requires the issuer URL to equal the URL the browser and the app use. Set `FA_OIDC_ISSUER`
and `FA_REDIRECT_URIS` (comma list of the app's callback URLs) before `up.sh` when the
app is not served from `http://127.0.0.1:18200`. An app deployed on a remote runtime reaches
these loopback services only if it runs on this host or the services are exposed under public
https paths.
