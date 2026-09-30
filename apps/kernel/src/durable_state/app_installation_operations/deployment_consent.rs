//! Protocol 367: the owner's single consent to deploy a workflow together with
//! its Apps, and the installations a deployment copy owns. The consent names
//! the exact releases (app, signer fingerprint, package and capabilities
//! digests); the install policy approves a deployment install only for those,
//! and only when the owner already approved the same capabilities
//! interactively. It is written on the installation writer, so the policy
//! reads it in the install decision's transaction.
use super::store::{identity, limit, now, sql};
use super::*;
use chariox_app_runtime::installation::ReleaseMetadata;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

/// The time the owner has to answer the consent prompt, and the time after
/// the owner's approval during which it approves the deployment's installs.
pub(crate) const CONSENT_TTL_MS: u64 = 5 * 60 * 1000;

/// An approval answered at `approved_ms` still approves installs.
fn approval_live(approved_ms: i64) -> Result<bool> {
    Ok(approved_ms.saturating_add(CONSENT_TTL_MS as i64) > now()?)
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_deployment_consents (
            owner_id TEXT NOT NULL, request_id TEXT NOT NULL, interaction_id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL, publication_id TEXT NOT NULL, deployment_id TEXT NOT NULL,
            release_id TEXT NOT NULL, package_digest TEXT NOT NULL, apps_json TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('pending','approved','declined')),
            expires_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL,
            PRIMARY KEY(owner_id,request_id));
         CREATE TABLE IF NOT EXISTS app_installation_deployments (
            installation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, deployment_id TEXT NOT NULL);",
    )
}

/// One consented release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsentedApp {
    pub(crate) app_id: String,
    pub(crate) publisher_id: String,
    pub(crate) publisher_key_fingerprint: String,
    pub(crate) package_digest: String,
    pub(crate) capabilities_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsentStatus {
    Pending,
    Approved,
    Declined,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeploymentConsent {
    pub(crate) request_id: String,
    pub(crate) interaction_id: String,
    pub(crate) session_id: String,
    pub(crate) publication_id: String,
    pub(crate) deployment_id: String,
    pub(crate) release_id: String,
    pub(crate) package_digest: String,
    pub(crate) apps: Vec<ConsentedApp>,
    pub(crate) status: ConsentStatus,
    pub(crate) expires_at_ms: u64,
}

impl DeploymentConsent {
    /// The same request names the same deployment facts.
    pub(crate) fn same_request(&self, other: &Self) -> bool {
        (
            &self.session_id,
            &self.publication_id,
            &self.deployment_id,
            &self.release_id,
            &self.package_digest,
        ) == (
            &other.session_id,
            &other.publication_id,
            &other.deployment_id,
            &other.release_id,
            &other.package_digest,
        )
    }
}

pub(super) enum ConsentCommand {
    /// Records a pending consent, or replays the request's existing one.
    Begin {
        owner: String,
        consent: DeploymentConsent,
        budget: AppOperationBudget,
    },
    /// The owner's answer; only a pending, unexpired consent changes.
    Decide {
        owner: String,
        interaction_id: String,
        approved: bool,
        budget: AppOperationBudget,
    },
}

pub(super) fn apply(connection: &mut Connection, command: ConsentCommand) -> Result<Reply> {
    match command {
        ConsentCommand::Begin {
            owner,
            consent,
            budget,
        } => {
            identity(&owner)?;
            for value in [
                &consent.request_id,
                &consent.interaction_id,
                &consent.session_id,
                &consent.publication_id,
                &consent.deployment_id,
                &consent.release_id,
                &consent.package_digest,
            ] {
                identity(value)?;
            }
            if consent.apps.is_empty() || consent.apps.len() > 64 {
                return Err(InstallOperationError::Invalid);
            }
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            if let Some(existing) = load(&tx, &owner, &consent.request_id)? {
                if !existing.same_request(&consent) {
                    return Err(InstallOperationError::Conflict);
                }
                return Ok(Reply::Consent(existing));
            }
            let apps =
                serde_json::to_string(&consent.apps).map_err(|_| InstallOperationError::Storage)?;
            sql(tx.execute(
                "INSERT INTO app_deployment_consents(owner_id,request_id,interaction_id,session_id,
                 publication_id,deployment_id,release_id,package_digest,apps_json,status,expires_ms,updated_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,?11)",
                params![
                    owner,
                    consent.request_id,
                    consent.interaction_id,
                    consent.session_id,
                    consent.publication_id,
                    consent.deployment_id,
                    consent.release_id,
                    consent.package_digest,
                    apps,
                    consent.expires_at_ms as i64,
                    now()?
                ],
            ))?;
            let value =
                load(&tx, &owner, &consent.request_id)?.ok_or(InstallOperationError::Storage)?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Consent(value))
        }
        ConsentCommand::Decide {
            owner,
            interaction_id,
            approved,
            budget,
        } => {
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let changed = sql(tx.execute(
                "UPDATE app_deployment_consents SET status=?1,updated_ms=?2
                 WHERE owner_id=?3 AND interaction_id=?4 AND status='pending' AND expires_ms>?2",
                params![
                    if approved { "approved" } else { "declined" },
                    now()?,
                    owner,
                    interaction_id
                ],
            ))?;
            if changed != 1 {
                return Err(InstallOperationError::Conflict);
            }
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Done)
        }
    }
}

pub(super) fn load(
    connection: &Connection,
    owner: &str,
    request_id: &str,
) -> Result<Option<DeploymentConsent>> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
    );
    let row: Option<Row> = sql(connection
        .query_row(
            "SELECT interaction_id,session_id,publication_id,deployment_id,release_id,
             package_digest,apps_json,status,expires_ms,updated_ms
             FROM app_deployment_consents WHERE owner_id=?1 AND request_id=?2",
            params![owner, request_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .optional())?;
    let Some((
        interaction_id,
        session_id,
        publication_id,
        deployment_id,
        release_id,
        package_digest,
        apps,
        status,
        expires,
        updated,
    )) = row
    else {
        return Ok(None);
    };
    let expires_at_ms = u64::try_from(expires).map_err(|_| InstallOperationError::Storage)?;
    Ok(Some(DeploymentConsent {
        request_id: request_id.into(),
        interaction_id,
        session_id,
        publication_id,
        deployment_id,
        release_id,
        package_digest,
        apps: serde_json::from_str(&apps).map_err(|_| InstallOperationError::Storage)?,
        status: match status.as_str() {
            "pending" if expires_at_ms <= now()? as u64 => ConsentStatus::Expired,
            "pending" => ConsentStatus::Pending,
            "approved" if !approval_live(updated)? => ConsentStatus::Expired,
            "approved" => ConsentStatus::Approved,
            "declined" => ConsentStatus::Declined,
            _ => return Err(InstallOperationError::Storage),
        },
        expires_at_ms,
    }))
}

/// The install policy: the owner approved `interaction_id` for this deployment
/// and exactly this release within `CONSENT_TTL_MS`, and approved its
/// capabilities interactively before.
pub(super) fn approves(
    tx: &Transaction<'_>,
    owner: &str,
    interaction_id: &str,
    deployment_id: &str,
    release: &ReleaseMetadata,
    publisher_key_fingerprint: &str,
) -> Result<bool> {
    let consent: Option<(String, String, i64)> = sql(tx
        .query_row(
            "SELECT status,apps_json,updated_ms FROM app_deployment_consents
             WHERE owner_id=?1 AND interaction_id=?2 AND deployment_id=?3",
            params![owner, interaction_id, deployment_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional())?;
    let Some((status, apps, answered_ms)) = consent else {
        return Ok(false);
    };
    let apps: Vec<ConsentedApp> =
        serde_json::from_str(&apps).map_err(|_| InstallOperationError::Storage)?;
    let consented = status == "approved"
        && approval_live(answered_ms)?
        && apps.iter().any(|app| {
            app.app_id == release.app_id
                && app.publisher_id == release.publisher_id
                && app.publisher_key_fingerprint == publisher_key_fingerprint
                && app.package_digest == release.package_digest
                && app.capabilities_digest == release.capabilities_digest
        });
    if !consented {
        return Ok(false);
    }
    sql(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_installation_updates u
         JOIN app_installations i ON i.installation_id=u.installation_id
         WHERE i.owner_id=?1 AND i.app_id=?2
         AND json_extract(u.record_json,'$.release.publisher_id')=?3
         AND json_extract(u.record_json,'$.release.capabilities_digest')=?4
         AND json_extract(u.record_json,'$.decision.status')='approved'
         AND json_extract(u.record_json,'$.decision.approval.authority_ref')='kernel_operation_human')",
        params![
            owner,
            release.app_id,
            release.publisher_id,
            release.capabilities_digest
        ],
        |r| r.get(0),
    ))
}

/// Tags a deployment copy's installation when it is staged.
pub(super) fn tag_installation(
    tx: &Transaction<'_>,
    owner: &str,
    installation_id: &str,
    deployment_id: &str,
) -> Result<()> {
    sql(tx.execute(
        "INSERT INTO app_installation_deployments(installation_id,owner_id,deployment_id)
         VALUES(?1,?2,?3) ON CONFLICT(installation_id) DO NOTHING",
        params![installation_id, owner, deployment_id],
    ))?;
    Ok(())
}

/// P1.20: a deployment updates only its own copy.
pub(super) fn require_copy_of(
    tx: &Transaction<'_>,
    owner: &str,
    installation_id: &str,
    deployment_id: &str,
) -> Result<()> {
    let tagged: bool = sql(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_installation_deployments
         WHERE installation_id=?1 AND owner_id=?2 AND deployment_id=?3)",
        params![installation_id, owner, deployment_id],
        |r| r.get(0),
    ))?;
    if tagged {
        Ok(())
    } else {
        Err(InstallOperationError::Invalid)
    }
}

impl DurableKernelStateStore {
    /// P1.20: the interaction of the owner's latest approved consent to run
    /// exactly this release of the deployment with its Apps.
    pub(crate) fn approved_deployment_consent(
        &self,
        owner: &str,
        deployment_id: &str,
        release_id: &str,
        package_digest: &str,
    ) -> Result<Option<String>> {
        let connection = self
            .lock_connection("durable_state.approved_deployment_consent")
            .map_err(|_| InstallOperationError::Storage)?;
        sql(connection
            .query_row(
                "SELECT interaction_id FROM app_deployment_consents
                 WHERE owner_id=?1 AND deployment_id=?2 AND release_id=?3 AND package_digest=?4
                 AND status='approved' ORDER BY updated_ms DESC, rowid DESC LIMIT 1",
                params![owner, deployment_id, release_id, package_digest],
                |r| r.get(0),
            )
            .optional())
    }

    /// Protocol 368: whether the owner already approved deploying this
    /// deployment with exactly these App releases (for any of its releases).
    pub(crate) fn deployment_apps_approved_before(
        &self,
        owner: &str,
        deployment_id: &str,
        apps: &[ConsentedApp],
    ) -> Result<bool> {
        let connection = self
            .lock_connection("durable_state.deployment_apps_approved_before")
            .map_err(|_| InstallOperationError::Storage)?;
        let mut statement = sql(connection.prepare(
            "SELECT apps_json FROM app_deployment_consents
             WHERE owner_id=?1 AND deployment_id=?2 AND status='approved'",
        ))?;
        let sorted = |mut apps: Vec<ConsentedApp>| {
            apps.sort_by(|a, b| {
                (&a.app_id, &a.package_digest).cmp(&(&b.app_id, &b.package_digest))
            });
            apps
        };
        let wanted = sorted(apps.to_vec());
        let rows =
            sql(statement.query_map(params![owner, deployment_id], |r| r.get::<_, String>(0)))?;
        for row in rows {
            let prior: Vec<ConsentedApp> =
                serde_json::from_str(&sql(row)?).map_err(|_| InstallOperationError::Storage)?;
            if sorted(prior) == wanted {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn begin_deployment_consent(
        &self,
        owner: &str,
        consent: DeploymentConsent,
        budget: AppOperationBudget,
    ) -> Result<DeploymentConsent> {
        match self.first_install(Command::Consent(ConsentCommand::Begin {
            owner: owner.into(),
            consent,
            budget,
        }))? {
            Reply::Consent(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }

    pub(crate) fn decide_deployment_consent(
        &self,
        owner: &str,
        interaction_id: &str,
        approved: bool,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.first_install(Command::Consent(ConsentCommand::Decide {
            owner: owner.into(),
            interaction_id: interaction_id.into(),
            approved,
            budget,
        }))?;
        Ok(())
    }

    pub(crate) fn deployment_consent(
        &self,
        owner: &str,
        request_id: &str,
    ) -> Result<Option<DeploymentConsent>> {
        let connection = self
            .lock_connection("durable_state.deployment_consent")
            .map_err(|_| InstallOperationError::Storage)?;
        load(&connection, owner, request_id)
    }

    /// The owner's deployment copy installations and their deployments.
    pub(crate) fn app_installation_deployments(
        &self,
        owner: &str,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let connection = self
            .lock_connection("durable_state.app_installation_deployments")
            .map_err(|_| InstallOperationError::Storage)?;
        let mut statement = sql(connection.prepare(
            "SELECT installation_id,deployment_id FROM app_installation_deployments WHERE owner_id=?1",
        ))?;
        let rows = sql(statement.query_map([owner], |r| Ok((r.get(0)?, r.get(1)?))))?;
        sql(rows.collect())
    }

    #[cfg(test)]
    pub(crate) fn fixture_tag_app_installation(
        &self,
        owner: &str,
        installation: &str,
        deployment: &str,
    ) {
        rusqlite::Connection::open(self.path()).unwrap()
            .execute(
                "INSERT INTO app_installation_deployments(installation_id,owner_id,deployment_id) VALUES(?1,?2,?3)",
                params![installation, owner, deployment],
            )
            .unwrap();
    }

    /// Ends both the answer window and an approval's install window.
    #[cfg(test)]
    pub(crate) fn fixture_expire_deployment_consent(&self, owner: &str, request_id: &str) {
        rusqlite::Connection::open(self.path()).unwrap()
            .execute(
                "UPDATE app_deployment_consents SET expires_ms=0,updated_ms=0 WHERE owner_id=?1 AND request_id=?2",
                params![owner, request_id],
            )
            .unwrap();
    }
}
