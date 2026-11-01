#!/bin/bash
# Start the FA regression fixtures: Mailpit, dex (local OIDC), Postgres, stripe-mock. Loopback only.
# Host needs only docker and node: dex's bcrypt hashes come from the stack's own Postgres (pgcrypto).
set -euo pipefail
cd "$(dirname "$0")"; . ./env.sh
umask 077; mkdir -p "$FA_STATE_DIR"
if [ ! -f "$FA_STATE_DIR/secrets.env" ]; then
  node -e "const c=require('node:crypto');console.log(['FA_DB_PASSWORD','FA_OIDC_CLIENT_SECRET','FA_ALICE_PASSWORD','FA_BOB_PASSWORD'].map(k=>k+'='+c.randomBytes(18).toString('base64url')).join('\n'))" > "$FA_STATE_DIR/secrets.env"
fi
set -a; . "$FA_STATE_DIR/secrets.env"; set +a
docker compose -f compose.yml up -d postgres
until docker exec amunblock-fa-postgres pg_isready -q -h 127.0.0.1 -U app -d app; do sleep 1; done
# Passwords go to psql on stdin and the local socket is trusted, so no secret reaches an argv.
hashes=$(printf "create extension if not exists pgcrypto;\nselect crypt('%s', gen_salt('bf', 10));\nselect crypt('%s', gen_salt('bf', 10));\n" "$FA_ALICE_PASSWORD" "$FA_BOB_PASSWORD" \
  | docker exec -i amunblock-fa-postgres psql -q -U app -d postgres -tA -v ON_ERROR_STOP=1)
FA_ALICE_HASH=$(sed -n 1p <<<"$hashes") FA_BOB_HASH=$(sed -n 2p <<<"$hashes") node - "$FA_STATE_DIR" "$FA_OIDC_ISSUER" "$FA_REDIRECT_URIS" <<'JS'
const [d, issuer, uris] = process.argv.slice(2), e = process.env
const user = (name, hash) => ({ email: `${name}@fa.invalid`, hash, username: name, userID: `fa-${name}` })
const fs = require('node:fs')
fs.writeFileSync(d + '/dex.yaml', JSON.stringify({
  issuer, storage: { type: 'memory' }, web: { http: '0.0.0.0:5556' }, oauth2: { skipApprovalScreen: true }, enablePasswordDB: true,
  staticClients: [{ id: 'fa-app', name: 'FA app', secret: e.FA_OIDC_CLIENT_SECRET, redirectURIs: uris.split(',') }],
  staticPasswords: [user('alice', e.FA_ALICE_HASH), user('bob', e.FA_BOB_HASH)],
}))
fs.chmodSync(d + '/dex.yaml', 0o644) // the dex image runs as non-root; umask 077 would make it 0600
JS
docker compose -f compose.yml up -d
echo "up: mail smtp $FA_PORT_MAIL_SMTP ui $FA_PORT_MAIL_UI, oidc $FA_OIDC_ISSUER, postgres $FA_PORT_DB, stripe-mock $FA_PORT_STRIPE (secrets in $FA_STATE_DIR/secrets.env)"
