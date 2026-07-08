use super::*;

pub(super) fn external_provider_import_session_alias(
    external: &ExternalProviderSessionRecord,
    requested_alias: Option<&str>,
) -> String {
    let alias_source = requested_alias
        .or(external.title.as_deref())
        .or(external.first_prompt_preview.as_deref())
        .unwrap_or(&external.provider_session_id);
    let mut base = session_alias_slug(alias_source).unwrap_or_else(|| {
        session_alias_slug(&format!(
            "{}-{}",
            external.provider, external.provider_session_id
        ))
        .unwrap_or_else(|| "unattached_agent".to_string())
    });
    let suffix = session_alias_slug(&external.provider_session_id)
        .map(|slug| short_alias_suffix(&slug))
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or_else(|| "imported".to_string());
    let reserved = suffix.len().saturating_add(1);
    if base.len().saturating_add(reserved) > EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN {
        base.truncate(EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN.saturating_sub(reserved));
        base = base.trim_matches(['-', '_']).to_string();
    }
    if base.is_empty() {
        suffix
    } else if base.ends_with(&format!("-{suffix}")) || base == suffix {
        base
    } else {
        format!("{base}-{suffix}")
    }
}

pub(super) fn session_alias_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_separator = false;
    for char in input.trim().chars().flat_map(|char| char.to_lowercase()) {
        if char.is_ascii_lowercase() || char.is_ascii_digit() {
            slug.push(char);
            previous_separator = false;
        } else if matches!(char, '-' | '_' | ' ' | '\t' | '\n' | '\r') {
            if !slug.is_empty() && !previous_separator {
                slug.push('-');
                previous_separator = true;
            }
        }
    }
    let slug = slug.trim_matches(['-', '_']).to_string();
    (!slug.is_empty()).then_some(slug)
}

pub(super) fn short_alias_suffix(slug: &str) -> String {
    const SHORT_SUFFIX_LEN: usize = 12;
    if slug.len() <= SHORT_SUFFIX_LEN {
        slug.to_string()
    } else {
        slug[slug.len() - SHORT_SUFFIX_LEN..].to_string()
    }
}
