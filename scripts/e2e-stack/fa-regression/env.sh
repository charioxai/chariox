# Sourced by up/down/status/verify. Secrets live only in $FA_STATE_DIR/secrets.env (mode 0600), never in the repository.
FA_PORT_BASE=${FA_PORT_BASE:-18100}
export FA_PORT_MAIL_SMTP=$FA_PORT_BASE FA_PORT_MAIL_UI=$((FA_PORT_BASE+1)) FA_PORT_OIDC=$((FA_PORT_BASE+2)) FA_PORT_DB=$((FA_PORT_BASE+3)) FA_PORT_STRIPE=$((FA_PORT_BASE+4))
export FA_STATE_DIR=${FA_STATE_DIR:-$HOME/.chariox/dev/amunblock/fa}
export FA_OIDC_ISSUER=${FA_OIDC_ISSUER:-http://127.0.0.1:$FA_PORT_OIDC/dex}
export FA_REDIRECT_URIS=${FA_REDIRECT_URIS:-http://127.0.0.1:18200/callback}
export COMPOSE_PROJECT_NAME=amunblock-fa
