//! Account sign-in uses the existing Inbox notices and their read/snooze state.
use super::*;
use crate::account_profile::ProviderAccountProfile;
use crate::local::PersonInboxNoticeAction;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

const ORIGIN: &str = "provider-account:";
const TARGET: &str = "chariox://provider-sign-in/";

fn card_id(profile: &ProviderAccountProfile) -> String {
    format!(
        "{ORIGIN}{}:{}:{}",
        profile.owner_user_id, profile.provider, profile.profile_id
    )
}

/// Only kernel notices with the reserved origin may project an executable action.
/// An agent's ordinary link remains an ordinary link, even with the same URL.
pub(super) fn action(notice: &StoredNotice) -> Option<PersonInboxNoticeAction> {
    if !notice.session_id.is_empty() || !notice.agent_id.starts_with(ORIGIN) {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(notice.link.as_deref()?.strip_prefix(TARGET)?)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl KernelRuntimeState {
    pub(crate) fn clear_provider_account_sign_in_notice(
        &self,
        profile: &ProviderAccountProfile,
    ) -> Result<(), DaemonError> {
        let store = self.durable_state_store();
        let id = card_id(profile);
        if store
            .person_inbox_state(&profile.owner_user_id, unix_epoch_ms())?
            .notices
            .iter()
            .any(|notice| notice.card_id == id)
        {
            store.remove_person_notice(&profile.owner_user_id, &id)?;
            self.record_person_inbox_change();
        }
        Ok(())
    }

    pub(crate) async fn reconcile_provider_account_sign_in_notice(
        &self,
        profile: &ProviderAccountProfile,
    ) -> Result<(), DaemonError> {
        if profile.auth_error.is_some() {
            return self.notify_provider_account_sign_in(profile, false).await;
        }
        if profile.auth_state == crate::account_profile::ProviderAccountAuthState::Authenticated
            && self
                .durable_state_store()
                .person_inbox_state(&profile.owner_user_id, unix_epoch_ms())?
                .notices
                .iter()
                .any(|notice| {
                    notice.card_id == card_id(profile)
                        && notice.priority == PersonInboxPriority::High
                })
        {
            self.clear_provider_account_sign_in_notice(profile)?;
        }
        Ok(())
    }
    pub(crate) async fn notify_provider_account_sign_in(
        &self,
        profile: &ProviderAccountProfile,
        copied: bool,
    ) -> Result<(), DaemonError> {
        let copied = copied && profile.auth_error.is_none();
        let config = self.config_snapshot().await;
        let machine = config
            .machine_name()
            .unwrap_or_else(|| config.host_machine_id.clone());
        let title = if copied {
            format!("{} was copied to {machine}", profile.label)
        } else {
            format!("Sign in again on {machine}")
        };
        let body = if copied {
            "Sign in there once so the copies don't log each other out".to_string()
        } else {
            profile
                .auth_error
                .clone()
                .unwrap_or_else(|| format!("Sign in again to {} on {machine}", profile.label))
        };
        let action = PersonInboxNoticeAction::ProviderSignIn {
            kernel_id: config.daemon_id.clone(),
            provider: profile.provider.clone(),
            account_profile: profile.profile_id.clone(),
            label: if copied {
                format!("Sign in on {machine}")
            } else {
                format!("Sign in again on {machine}")
            },
        };
        let now = unix_epoch_ms();
        let card_id = card_id(profile);
        let store = self.durable_state_store();
        if store
            .person_inbox_state(&profile.owner_user_id, now)?
            .notices
            .iter()
            .any(|notice| {
                notice.card_id == card_id
                    && notice.title == title
                    && notice.body.as_ref() == Some(&body)
            })
        {
            return Ok(());
        }
        store.replace_person_notice(StoredNotice {
            card_id, owner_user_id: profile.owner_user_id.clone(), session_id: String::new(),
            agent_id: format!("{ORIGIN}{}", profile.profile_id), title, body: Some(body),
            link: Some(format!("{TARGET}{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&action)
                .map_err(|_| refused("The account sign-in action could not be saved; refresh the account and try again"))?))),
            priority: if copied { PersonInboxPriority::Normal } else { PersonInboxPriority::High },
            created_at_ms: now, read: false, folded: 0,
        })?;
        self.record_person_inbox_change();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_agent_links_cannot_become_provider_sign_in_actions() {
        let expected = PersonInboxNoticeAction::ProviderSignIn {
            kernel_id: "target".into(),
            provider: "codex".into(),
            account_profile: "work".into(),
            label: "Sign in on target".into(),
        };
        let mut notice = StoredNotice {
            card_id: "test".into(),
            owner_user_id: "owner".into(),
            session_id: "room".into(),
            agent_id: "provider-account:work".into(),
            title: "Sign in".into(),
            body: None,
            link: Some(format!(
                "{TARGET}{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&expected).unwrap())
            )),
            priority: PersonInboxPriority::High,
            created_at_ms: 1,
            read: false,
            folded: 0,
        };
        assert_eq!(action(&notice), None, "agent links always belong to a Room");
        notice.session_id.clear();
        notice.agent_id = "ordinary-agent".into();
        assert_eq!(action(&notice), None);
        notice.agent_id = "provider-account:work".into();
        assert_eq!(action(&notice), Some(expected));
    }
}
