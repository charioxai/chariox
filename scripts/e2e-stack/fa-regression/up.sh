#!/bin/bash
# Start the FA regression fixtures: Mailpit, dex (local OIDC), Postgres, stripe-mock. Loopback only.
set -euo pipefail
cd "$(dirname "$0")"; . ./env.sh
umask 077; mkdir -p "$FA_STATE_DIR"
if [ ! -f "$FA_STATE_DIR/secrets.env" ]; then
  python3 - "$FA_STATE_DIR" <<'PY'
import secrets,sys
d=sys.argv[1]
open(d+'/secrets.env','w').write(''.join(f'{k}={secrets.token_urlsafe(18)}\n' for k in ('FA_DB_PASSWORD','FA_OIDC_CLIENT_SECRET','FA_ALICE_PASSWORD','FA_BOB_PASSWORD')))
PY
fi
set -a; . "$FA_STATE_DIR/secrets.env"; set +a
python3 - "$FA_STATE_DIR" "$FA_OIDC_ISSUER" "$FA_PORT_OIDC" "$FA_REDIRECT_URIS" <<'PY'
import bcrypt,os,sys,json
d,issuer,port,uris=sys.argv[1:5]
h=lambda k:bcrypt.hashpw(os.environ[k].encode(),bcrypt.gensalt(10)).decode()
cfg={'issuer':issuer,'storage':{'type':'memory'},'web':{'http':'127.0.0.1:'+port},'oauth2':{'skipApprovalScreen':True},'enablePasswordDB':True,
 'staticClients':[{'id':'fa-app','name':'FA app','secret':os.environ['FA_OIDC_CLIENT_SECRET'],'redirectURIs':uris.split(',')}],
 'staticPasswords':[{'email':'alice@fa.invalid','hash':h('FA_ALICE_PASSWORD'),'username':'alice','userID':'fa-alice'},{'email':'bob@fa.invalid','hash':h('FA_BOB_PASSWORD'),'username':'bob','userID':'fa-bob'}]}
open(d+'/dex.yaml','w').write(json.dumps(cfg)); os.chmod(d+'/dex.yaml',0o644)
PY
docker compose -f compose.yml up -d
echo "up: mail smtp $FA_PORT_MAIL_SMTP ui $FA_PORT_MAIL_UI, oidc $FA_OIDC_ISSUER, postgres $FA_PORT_DB, stripe-mock $FA_PORT_STRIPE (secrets in $FA_STATE_DIR/secrets.env)"
