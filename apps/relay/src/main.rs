use std::collections::BTreeMap;

use arroba_relay::{RelayAuthVerifier, RelayConfig, RelayServer};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RelayConfig::load_from_env()?;
    let server = if let Some(verifier) = scoped_verifier_from_env() {
        RelayServer::with_auth_verifier(config.clone(), verifier)
    } else {
        RelayServer::new(config.clone())
    };
    println!("arroba relay listening on {}:{}", config.host, config.port);
    server.run().await?;
    Ok(())
}

fn scoped_verifier_from_env() -> Option<RelayAuthVerifier> {
    let issuer = std::env::var("ARROBA_RELAY_SCOPED_ISSUER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let secret = std::env::var("ARROBA_RELAY_SCOPED_HMAC_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(issuer, secret)]),
        None,
    ))
}
