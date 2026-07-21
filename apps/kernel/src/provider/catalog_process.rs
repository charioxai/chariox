use std::ops::Deref;
use std::process::Child;

#[derive(Debug)]
pub struct ProviderCatalogEndpoint {
    endpoint: String,
    managed_child: Option<Child>,
}

impl ProviderCatalogEndpoint {
    pub(crate) fn existing(endpoint: String) -> Self {
        Self {
            endpoint,
            managed_child: None,
        }
    }

    pub(crate) fn managed(endpoint: String, child: Child) -> Self {
        Self {
            endpoint,
            managed_child: Some(child),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn into_persistent_endpoint(mut self) -> String {
        self.managed_child.take();
        std::mem::take(&mut self.endpoint)
    }
}

impl Deref for ProviderCatalogEndpoint {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Drop for ProviderCatalogEndpoint {
    fn drop(&mut self) {
        let Some(mut child) = self.managed_child.take() else {
            return;
        };
        let _ = crate::runtime::process_health::terminate_process_tree(child.id());
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::ProviderCatalogEndpoint;

    #[test]
    fn managed_catalog_endpoint_terminates_its_process_tree_on_drop() {
        let child = Command::new("sh")
            .args(["-c", "sleep 60"])
            .spawn()
            .expect("catalog fixture should start");
        let pid = child.id();

        drop(ProviderCatalogEndpoint::managed(
            "http://127.0.0.1:1".to_string(),
            child,
        ));

        assert!(!crate::runtime::process_health::process_running(pid));
    }
}
