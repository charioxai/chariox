use super::CodexClient;

pub fn codex_endpoint_is_healthy(endpoint: &str) -> bool {
    codex_readyz_is_healthy(endpoint)
        || CodexClient::new("catalog", endpoint)
            .and_then(|client| client.connect_initialized())
            .is_ok()
}

fn codex_readyz_is_healthy(endpoint: &str) -> bool {
    let Ok(mut url) = url::Url::parse(endpoint) else {
        return false;
    };
    match url.scheme() {
        "ws" => {
            let _ = url.set_scheme("http");
        }
        "wss" => {
            let _ = url.set_scheme("https");
        }
        "http" | "https" => {}
        _ => return false,
    }
    url.set_path("/readyz");
    url.set_query(None);
    match ureq::get(url.as_str()).call() {
        Ok(response) => response.status() == 200,
        Err(_) => false,
    }
}
