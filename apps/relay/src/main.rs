use arroba_relay::{RelayConfig, RelayServer};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RelayConfig::load_from_env()?;
    let server = RelayServer::new(config.clone());
    println!("arroba relay listening on {}:{}", config.host, config.port);
    server.run().await?;
    Ok(())
}
