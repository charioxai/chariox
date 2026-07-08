use super::*;

pub(super) fn provider_transcript_path_index()
-> &'static Mutex<BTreeMap<String, ExternalProviderTranscriptIndexEntry>> {
    PROVIDER_TRANSCRIPT_PATH_INDEX.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn provider_transcript_discovery_path_index()
-> &'static Mutex<BTreeMap<(String, PathBuf), ExternalProviderTranscriptDiscoveryPathEntry>> {
    PROVIDER_TRANSCRIPT_DISCOVERY_PATH_INDEX.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn provider_transcript_path_index_key(
    provider: &str,
    provider_session_id: &str,
) -> String {
    format!("{provider}:{provider_session_id}")
}

pub(super) fn cached_provider_transcript_path(
    provider: &str,
    provider_session_id: &str,
) -> Option<PathBuf> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let path = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .map(|entry| entry.path.clone())?;
    path.is_file().then_some(path)
}

pub(super) fn cached_provider_transcript_path_in_root(
    provider: &str,
    provider_session_id: &str,
    root: &Path,
) -> Option<PathBuf> {
    let path = cached_provider_transcript_path(provider, provider_session_id)?;
    path.starts_with(root).then_some(path)
}

pub(super) fn remember_provider_transcript_path(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
) {
    let Some(fingerprint) = provider_transcript_file_fingerprint(path) else {
        return;
    };
    remember_provider_transcript_path_with_fingerprint(
        provider,
        provider_session_id,
        path,
        fingerprint,
    );
}

pub(super) fn remember_provider_transcript_path_with_fingerprint(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    if let Ok(mut index) = provider_transcript_path_index().lock() {
        let previous = index.get(&key).filter(|existing| {
            existing.path == path && fingerprint.len >= existing.last_observed_offset
        });
        let observed_turns = previous.and_then(|existing| existing.observed_turns.clone());
        let discovery_record = previous.and_then(|existing| {
            (existing.len == fingerprint.len
                && existing.modified_at_ms == fingerprint.modified_at_ms)
                .then(|| existing.discovery_record.clone())
                .flatten()
        });
        let last_observed_offset = observed_turns
            .as_ref()
            .and_then(|_| previous.map(|existing| existing.last_observed_offset))
            .unwrap_or(0);
        index.insert(
            key,
            ExternalProviderTranscriptIndexEntry {
                provider_session_id: provider_session_id.to_string(),
                path: path.to_path_buf(),
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                discovery_record,
                last_observed_offset,
                observed_turns,
            },
        );
    }
}

pub(super) fn cached_provider_observed_turns(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let entry = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .cloned()?;
    (entry.provider_session_id == provider_session_id
        && entry.path == path
        && entry.len == fingerprint.len
        && entry.modified_at_ms == fingerprint.modified_at_ms)
        .then(|| entry.observed_turns)
        .flatten()
}

pub(super) fn cached_provider_discovery_record_for_path(
    provider: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> Option<ExternalProviderSessionRecord> {
    let key = (provider.to_string(), path.to_path_buf());
    provider_transcript_discovery_path_index()
        .lock()
        .ok()?
        .get(&key)
        .filter(|entry| {
            entry.record.provider == provider
                && entry.len == fingerprint.len
                && entry.modified_at_ms == fingerprint.modified_at_ms
        })
        .map(|entry| entry.record.clone())
}

pub(super) fn cached_provider_observed_turns_for_path(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let entry = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .cloned()?;
    (entry.provider_session_id == provider_session_id && entry.path == path)
        .then(|| entry.observed_turns)
        .flatten()
}

pub(super) fn cached_provider_observed_transcript_for_path(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
) -> Option<CachedProviderObservedTranscript> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let entry = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .cloned()?;
    let observed_turns = (entry.provider_session_id == provider_session_id && entry.path == path)
        .then(|| entry.observed_turns)
        .flatten()?;
    Some(CachedProviderObservedTranscript {
        last_observed_offset: entry.last_observed_offset,
        observed_turns,
    })
}

pub(super) fn cached_provider_transcript_identity_matches(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> bool {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    provider_transcript_path_index()
        .lock()
        .ok()
        .and_then(|index| index.get(&key).cloned())
        .is_some_and(|entry| {
            entry.provider_session_id == provider_session_id
                && entry.path == path
                && fingerprint.len >= entry.last_observed_offset
        })
}

pub(super) fn remember_provider_observed_turns(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
    turns: Vec<ObservedExternalProviderTurn>,
) {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    if let Ok(mut index) = provider_transcript_path_index().lock() {
        let discovery_record = index.get(&key).and_then(|existing| {
            (existing.path == path
                && existing.len == fingerprint.len
                && existing.modified_at_ms == fingerprint.modified_at_ms)
                .then(|| existing.discovery_record.clone())
                .flatten()
        });
        index.insert(
            key,
            ExternalProviderTranscriptIndexEntry {
                provider_session_id: provider_session_id.to_string(),
                path: path.to_path_buf(),
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                discovery_record,
                last_observed_offset: fingerprint.len,
                observed_turns: Some(turns),
            },
        );
    }
}

pub(super) fn remember_provider_discovery_record(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
    record: ExternalProviderSessionRecord,
) {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let path_key = (provider.to_string(), path.to_path_buf());
    if let Ok(mut index) = provider_transcript_path_index().lock() {
        let previous = index.get(&key).filter(|existing| {
            existing.path == path && fingerprint.len >= existing.last_observed_offset
        });
        let observed_turns = previous.and_then(|existing| existing.observed_turns.clone());
        let last_observed_offset = observed_turns
            .as_ref()
            .and_then(|_| previous.map(|existing| existing.last_observed_offset))
            .unwrap_or(0);
        index.insert(
            key,
            ExternalProviderTranscriptIndexEntry {
                provider_session_id: provider_session_id.to_string(),
                path: path.to_path_buf(),
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                discovery_record: Some(record.clone()),
                last_observed_offset,
                observed_turns,
            },
        );
    }
    if let Ok(mut index) = provider_transcript_discovery_path_index().lock() {
        index.insert(
            path_key,
            ExternalProviderTranscriptDiscoveryPathEntry {
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                record,
            },
        );
    }
}
