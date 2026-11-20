# Sourced by up/down/status/verify. Secrets live only in $FA_STATE_DIR/secrets.env (mode 0600), never in the repository.
FA_PORT_BASE=${FA_PORT_BASE:-18100}
export FA_PORT_MAIL_SMTP=$FA_PORT_BASE FA_PORT_MAIL_UI=$((FA_PORT_BASE+1)) FA_PORT_OIDC=$((FA_PORT_BASE+2)) FA_PORT_DB=$((FA_PORT_BASE+3)) FA_PORT_STRIPE=$((FA_PORT_BASE+4))
export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-amunblock-fa}
if [[ ! "$COMPOSE_PROJECT_NAME" =~ ^[a-z0-9][a-z0-9_-]*$ ]]; then
  printf 'COMPOSE_PROJECT_NAME must be a valid Compose project name\n' >&2
  return 2
fi
if [[ "$COMPOSE_PROJECT_NAME" == amunblock-fa ]]; then
  export FA_STATE_DIR=${FA_STATE_DIR:-$HOME/.chariox/dev/amunblock/fa}
else
  export FA_STATE_DIR=${FA_STATE_DIR:-$HOME/.chariox/dev/amunblock/$COMPOSE_PROJECT_NAME}
fi
export FA_OIDC_ISSUER=${FA_OIDC_ISSUER:-http://127.0.0.1:$FA_PORT_OIDC/dex}
export FA_REDIRECT_URIS=${FA_REDIRECT_URIS:-http://127.0.0.1:18200/callback}
