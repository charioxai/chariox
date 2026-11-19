//! MP-08 / MP-10 / MP-11: model previews retain the admitted file/network boundary.
use super::catalog::failure;
use crate::error::DaemonError;
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::Path,
};

#[derive(Clone, Copy)]
pub(crate) enum SkillSourceAccess<'a> {
    Owner,
    Agent { workspace: Option<&'a Path> },
}
impl SkillSourceAccess<'_> {
    pub(super) fn download_agent(self, source: &str) -> Result<ureq::Agent, DaemonError> {
        let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(30));
        if matches!(self, Self::Agent { .. }) {
            let url = url::Url::parse(source).map_err(|_| failure("invalid skill source URL"))?;
            if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
                return Err(failure(
                    "agent skill downloads require a public HTTPS URL without embedded credentials",
                ));
            }
            // ureq resolves each direct connection and redirect with this
            // resolver. Ambient proxy credentials/configuration are not used.
            builder = builder
                .https_only(true)
                .try_proxy_from_env(false)
                .resolver(resolve_public_addresses);
        }
        Ok(builder.build())
    }
    /// An agent's repository is fetched by the home kernel's own `git`, so it
    /// keeps the boundary of agent downloads: public HTTPS hosts only, never a
    /// local path or a private network address of the home machine.
    pub(crate) fn repository_url(self, url: &str) -> Result<(), DaemonError> {
        if matches!(self, Self::Owner) {
            return Ok(());
        }
        let refused = || {
            failure("agents import and update only public https repositories (https://host/owner/repo); the owner can use other repository sources from Skills")
        };
        let url = url::Url::parse(url).map_err(|_| refused())?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return Err(refused());
        }
        let addresses = url
            .socket_addrs(|| Some(443))
            .map_err(|_| failure("cannot resolve the repository host; check the URL"))?;
        validate_public_addresses(addresses.into_iter().take(33).collect())
            .map(|_| ())
            .map_err(|_| refused())
    }
}
fn resolve_public_addresses(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    let addresses = netloc.to_socket_addrs()?.take(33).collect();
    validate_public_addresses(addresses)
}
fn validate_public_addresses(addresses: Vec<SocketAddr>) -> io::Result<Vec<SocketAddr>> {
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses.iter().any(|address| {
            !crate::runtime::aegs_network_policy::is_globally_reachable_aegs_destination(
                address.ip(),
            )
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "skill source must resolve only to public addresses",
        ));
    }
    Ok(addresses)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_agent_download_rejects_private_or_mixed_dns_and_embedded_credentials() {
        for addresses in [
            vec![],
            vec!["127.0.0.1:443".parse().unwrap()],
            vec!["169.254.169.254:443".parse().unwrap()],
            vec!["1.1.1.1:443".parse().unwrap(), "[::1]:443".parse().unwrap()],
        ] {
            assert!(validate_public_addresses(addresses).is_err());
        }
        assert!(validate_public_addresses(vec!["1.1.1.1:443".parse().unwrap()]).is_ok());
        for url in [
            "http://example.com/SKILL.md",
            "https://user:password@example.com/SKILL.md",
        ] {
            assert!(SkillSourceAccess::Agent { workspace: None }
                .download_agent(url)
                .is_err());
        }
        for url in [
            "file:///srv/skills",
            "http://example.com/org/skills.git",
            "https://user:secret@example.com/org/skills.git",
            "https://127.0.0.1/org/skills.git",
            "https://10.0.0.8/org/skills.git",
            "https://[::1]/org/skills.git",
            "https://localhost/org/skills.git",
        ] {
            let error = SkillSourceAccess::Agent { workspace: None }
                .repository_url(url)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("secret"), "{error}");
        }
        assert!(SkillSourceAccess::Agent { workspace: None }
            .repository_url("https://1.1.1.1/org/skills.git")
            .is_ok());
        assert!(SkillSourceAccess::Owner
            .repository_url("file:///srv/skills")
            .is_ok());
        assert!(
            SkillSourceAccess::Owner
                .download_agent("http://127.0.0.1/SKILL.md")
                .is_ok(),
            "explicit owner imports preserve local/self-host URLs"
        );
    }
}
