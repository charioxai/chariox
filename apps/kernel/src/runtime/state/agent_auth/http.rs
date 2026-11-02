//! Keep credentials and response/error bodies inside the kernel. Disable redirects:
//! an AS redirect must never forward an assertion or claim token to another host.
use super::*;
use std::io::Read;
use zeroize::Zeroizing;

const MAX_RESPONSE: u64 = 1024 * 1024;

pub(super) fn endpoint(raw: &str) -> Result<url::Url, DaemonError> {
    let url = url::Url::parse(raw).map_err(|_| error("auth.md endpoint must be a URL"))?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if host.is_empty()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(error("auth.md endpoints require HTTPS (HTTP is allowed only on loopback) and no URL credentials or fragment"));
    }
    Ok(url)
}

pub(super) struct AuthHttp {
    authority: Authority,
    agent: ureq::Agent,
}

pub(super) struct Reply {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Zeroizing<String>,
}

impl Reply {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, DaemonError> {
        serde_json::from_str(&self.body).map_err(|_| {
            error("auth.md service returned an invalid response; consult the service documentation")
        })
    }
    pub fn success(&self) -> Result<(), DaemonError> {
        if (200..300).contains(&self.status) {
            Ok(())
        } else {
            Err(error(format!("auth.md service returned HTTP {}; consult the service documentation before retrying", self.status)))
        }
    }
}

impl AuthHttp {
    pub fn new(authority: Authority) -> Self {
        Self {
            authority,
            agent: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(20))
                .build(),
        }
    }
    pub fn authority(&self) -> &Authority {
        &self.authority
    }
    pub async fn get(&self, target: &str) -> Result<Reply, DaemonError> {
        self.send(target, None).await
    }
    pub async fn post_json<T: serde::Serialize>(
        &self,
        target: &str,
        body: &T,
    ) -> Result<Reply, DaemonError> {
        let body = Zeroizing::new(
            serde_json::to_string(body).map_err(|_| error("could not encode auth.md request"))?,
        );
        self.send(target, Some(("application/json", body))).await
    }
    pub async fn post_form(
        &self,
        target: &str,
        pairs: &[(&str, &str)],
    ) -> Result<Reply, DaemonError> {
        let body = Zeroizing::new(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs.iter().copied())
                .finish(),
        );
        self.send(target, Some(("application/x-www-form-urlencoded", body)))
            .await
    }
    async fn send(
        &self,
        target: &str,
        body: Option<(&'static str, Zeroizing<String>)>,
    ) -> Result<Reply, DaemonError> {
        let target = endpoint(target)?;
        self.authority.check()?;
        let authority = self.authority.clone();
        let agent = self.agent.clone();
        tokio::task::spawn_blocking(move || {
            authority.check()?;
            let response = match body {
                Some((content_type, body)) => agent
                    .post(target.as_str())
                    .set("Content-Type", content_type)
                    .send_string(&body),
                None => agent.get(target.as_str()).call(),
            };
            let response = match response {
                Ok(response) | Err(ureq::Error::Status(_, response)) => response,
                Err(_) => {
                    return Err(error(
                        "auth.md service could not be reached; check its URL and try again",
                    ))
                }
            };
            let status = response.status();
            let retry_after = response
                .header("Retry-After")
                .and_then(|value| value.parse::<u64>().ok());
            let mut bytes = Zeroizing::new(Vec::new());
            response
                .into_reader()
                .take(MAX_RESPONSE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| error("auth.md service response could not be read"))?;
            if bytes.len() as u64 > MAX_RESPONSE {
                return Err(error("auth.md service response exceeded 1 MiB"));
            }
            let body = Zeroizing::new(
                String::from_utf8(std::mem::take(&mut *bytes))
                    .map_err(|_| error("auth.md service response is not UTF-8"))?,
            );
            authority.check()?;
            Ok(Reply {
                status,
                retry_after,
                body,
            })
        })
        .await
        .map_err(|_| error("auth.md HTTP task did not finish"))?
    }
}
