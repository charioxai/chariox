//! Remote managed-I/O relay state conversions.

use super::*;

pub(in crate::runtime::state) fn remote_managed_io_state(
    path: &PathBuf,
    content_text: Option<String>,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    crate::transport::relay_peer::RemoteManagedIoArtifactState {
        path: path.to_string_lossy().to_string(),
        exists: content_text.is_some(),
        domain: Some("text".to_string()),
        content_text,
        content_base64: None,
    }
}

pub(in crate::runtime::state) fn remote_managed_io_state_from_content(
    path: &PathBuf,
    content: Option<crate::io::ArtifactContent>,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    match content {
        Some(crate::io::ArtifactContent::Text(text)) => remote_managed_io_state(path, Some(text)),
        Some(crate::io::ArtifactContent::Bytes(bytes)) => {
            crate::transport::relay_peer::RemoteManagedIoArtifactState {
                path: path.to_string_lossy().to_string(),
                exists: true,
                domain: Some("opaque".to_string()),
                content_text: None,
                content_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            }
        }
        None => crate::transport::relay_peer::RemoteManagedIoArtifactState {
            path: path.to_string_lossy().to_string(),
            exists: false,
            domain: None,
            content_text: None,
            content_base64: None,
        },
    }
}

pub(in crate::runtime::state) fn remote_managed_io_state_from_content_with_domain(
    path: &PathBuf,
    content: Option<crate::io::ArtifactContent>,
    domain: crate::io::ArtifactDomainKind,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    let mut state = remote_managed_io_state_from_content(path, content);
    state.domain = Some(
        match domain {
            crate::io::ArtifactDomainKind::TextDocument => "text",
            crate::io::ArtifactDomainKind::StructuredDocument => "structured",
            crate::io::ArtifactDomainKind::OpaqueBlob => "opaque",
        }
        .to_string(),
    );
    state
}

pub(in crate::runtime::state) fn remote_managed_io_state_for_path<'a>(
    states: &'a [crate::transport::relay_peer::RemoteManagedIoArtifactState],
    path: &PathBuf,
) -> Option<&'a crate::transport::relay_peer::RemoteManagedIoArtifactState> {
    let expected = path.to_string_lossy();
    states.iter().find(|state| state.path == expected)
}

pub(in crate::runtime::state) fn remote_managed_io_content_from_state(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
    domain: crate::io::ArtifactDomainKind,
) -> Result<crate::io::ArtifactContent, DaemonError> {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument
        | crate::io::ArtifactDomainKind::StructuredDocument => Ok(
            crate::io::ArtifactContent::Text(state.content_text.clone().unwrap_or_default()),
        ),
        crate::io::ArtifactDomainKind::OpaqueBlob => {
            let bytes = match state.content_base64.as_deref() {
                Some(content_base64) => base64::engine::general_purpose::STANDARD
                    .decode(content_base64)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "remote_managed_io_content",
                        message: format!(
                            "forwarded opaque artifact state is not valid base64: {error}"
                        ),
                    })?,
                None => Vec::new(),
            };
            Ok(crate::io::ArtifactContent::Bytes(bytes))
        }
    }
}

pub(in crate::runtime::state) fn remote_managed_io_state_domain(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> crate::io::ArtifactDomainKind {
    if let Some(domain) = state.domain.as_deref() {
        if let Ok(domain) = KernelRuntimeOwnedState::managed_io_domain_from_arg(Some(domain)) {
            return domain;
        }
    }
    if state.content_base64.is_some() {
        crate::io::ArtifactDomainKind::OpaqueBlob
    } else {
        crate::io::ArtifactDomainKind::TextDocument
    }
}

pub(in crate::runtime::state) fn remote_managed_io_states_content_equal(
    left: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
    right: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> bool {
    left.exists == right.exists
        && left.content_text == right.content_text
        && left.content_base64 == right.content_base64
}

pub(in crate::runtime::state) fn remote_managed_io_text_snapshot_from_state(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> Option<ManagedIoTextSnapshot> {
    Some(ManagedIoTextSnapshot {
        existed: state.exists,
        text: state.content_text.clone().unwrap_or_default(),
    })
}
