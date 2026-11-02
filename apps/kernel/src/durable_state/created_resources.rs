//! MP-08 / MP-09 / MP-10 / MP-11 AU-11: the event and register row share one writer transaction.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::CreatedResource;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const EVENT: &str = "resource_register.changed";

#[derive(Serialize, Deserialize)]
struct Change {
    owner: String,
    resource: CreatedResource,
    expected: Option<CreatedResource>,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS created_resources (
        owner_id TEXT NOT NULL, project_id TEXT NOT NULL, resource_id TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL, payload TEXT NOT NULL, creation_json TEXT NOT NULL,
        PRIMARY KEY(owner_id, project_id, resource_id)
    );
    UPDATE created_resources SET payload=json_set(payload,'$.state','revoke_failed','$.last_http_status',NULL)
      WHERE json_extract(payload,'$.state')='revoking';",
    )
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    if kind != EVENT {
        return Ok(());
    }
    let change: Change =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let row = &change.resource;
    let encoded = serde_json::to_string(row).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let changed = if let Some(expected) = change.expected {
        let expected =
            serde_json::to_string(&expected).map_err(|_| rusqlite::Error::InvalidQuery)?;
        tx.execute(
            "UPDATE created_resources SET payload=?4
            WHERE owner_id=?1 AND project_id=?2 AND resource_id=?3
            AND payload=?5",
            params![change.owner, row.project_id, row.id, encoded, expected],
        )?
    } else {
        tx.execute(
            "INSERT INTO created_resources(owner_id,project_id,resource_id,created_at_ms,payload,creation_json)
            VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT DO NOTHING",
            params![
                change.owner,
                row.project_id,
                row.id,
                i64::try_from(row.created_at_ms).map_err(|_| rusqlite::Error::InvalidQuery)?,
                encoded
            ],
        )?
    };
    if changed != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn record_created_resource(
        &self,
        owner: &str,
        row: &CreatedResource,
    ) -> Result<CreatedResource, DaemonError> {
        if let Some((previous, creation)) =
            self.created_resource_record(owner, &row.project_id, &row.id)?
        {
            return same_resource(previous, creation, row);
        }
        match self.change_created_resource(owner, row, None) {
            Ok(()) => Ok(row.clone()),
            Err(error) => match self.created_resource_record(owner, &row.project_id, &row.id)? {
                Some((previous, creation)) => same_resource(previous, creation, row),
                None => Err(error),
            },
        }
    }

    pub(crate) fn change_created_resource(
        &self,
        owner: &str,
        row: &CreatedResource,
        expected: Option<&CreatedResource>,
    ) -> Result<(), DaemonError> {
        let change = Change {
            owner: owner.into(),
            resource: row.clone(),
            expected: expected.cloned(),
        };
        self.append_event(
            EVENT,
            Some(row.id.clone()),
            serde_json::to_value(change).map_err(|_| error("cannot encode resource metadata"))?,
        )?;
        Ok(())
    }

    pub(crate) fn created_resource(
        &self,
        owner: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<CreatedResource>, DaemonError> {
        Ok(self
            .created_resource_record(owner, project, id)?
            .map(|(current, _)| current))
    }

    fn created_resource_record(
        &self,
        owner: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<(CreatedResource, CreatedResource)>, DaemonError> {
        let payload: Option<(String, String)> = self.lock_connection("resource_register.read")?
            .query_row("SELECT payload,creation_json FROM created_resources WHERE owner_id=?1 AND project_id=?2 AND resource_id=?3",
                params![owner,project,id], |row| Ok((row.get(0)?,row.get(1)?))).optional()
            .map_err(|_| error("cannot read resource register"))?;
        payload
            .map(|(current, creation)| {
                let decode = |text: &str| {
                    serde_json::from_str(text).map_err(|_| error("resource metadata is invalid"))
                };
                Ok((decode(&current)?, decode(&creation)?))
            })
            .transpose()
    }

    pub(crate) fn created_resources(
        &self,
        owner: &str,
        project: &str,
    ) -> Result<Vec<CreatedResource>, DaemonError> {
        let connection = self.lock_connection("resource_register.list")?;
        let mut query = connection.prepare("SELECT payload FROM created_resources WHERE owner_id=?1 AND project_id=?2 ORDER BY created_at_ms,resource_id")
            .map_err(|_| error("cannot list resource register"))?;
        let rows = query
            .query_map(params![owner, project], |row| row.get::<_, String>(0))
            .map_err(|_| error("cannot list resource register"))?;
        rows.map(|row| {
            let payload = row.map_err(|_| error("cannot read resource metadata"))?;
            serde_json::from_str(&payload).map_err(|_| error("resource metadata is invalid"))
        })
        .collect()
    }
}

fn same_resource(
    previous: CreatedResource,
    creation: CreatedResource,
    row: &CreatedResource,
) -> Result<CreatedResource, DaemonError> {
    // Lost acknowledgements must not resurrect a revoked resource or change its provenance.
    if creation.project_id != row.project_id
        || creation.service != row.service
        || creation.kind != row.kind
        || creation.name != row.name
        || creation.url != row.url
        || creation.credential_id != row.credential_id
        || creation.created_by != row.created_by
        || creation.revoke != row.revoke
    {
        return Err(error(
            "request_id already records a different resource; use a new request_id",
        ));
    }
    Ok(previous)
}

fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "resource_register",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{CreatedResourceState, ResourceRevokeMethod};

    #[test]
    fn resource_register_survives_restart_and_fences_stale_revocation() {
        let worktree = crate::test_support::TestWorktree::new("created-resources");
        let path = worktree.path().join("state.sqlite3");
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        let mut row = CreatedResource {
            id: "key".into(),
            project_id: "project".into(),
            service: "https://example.com".into(),
            kind: "api_key".into(),
            name: "deploy".into(),
            url: None,
            credential_id: Some("handle".into()),
            created_by: "agent".into(),
            created_at_ms: 42,
            revoke: ResourceRevokeMethod::Url {
                url: "https://example.com/settings".into(),
            },
            state: CreatedResourceState::Active,
            revoked_at_ms: None,
            last_http_status: None,
        };
        store.record_created_resource("owner", &row).unwrap();
        assert!(store
            .created_resources("other-owner", "project")
            .unwrap()
            .is_empty());
        assert!(store
            .created_resources("owner", "other-project")
            .unwrap()
            .is_empty());
        let original = row.clone();
        let stale = row.clone();
        row.revoke = ResourceRevokeMethod::Url {
            url: "https://example.com/new-settings".into(),
        };
        store
            .change_created_resource("owner", &row, Some(&stale))
            .unwrap();
        assert_eq!(
            store.record_created_resource("owner", &original).unwrap(),
            row
        );
        let mut conflicting = original.clone();
        conflicting.revoke = ResourceRevokeMethod::Unavailable {
            reason: "different creation request".into(),
        };
        assert!(store
            .record_created_resource("owner", &conflicting)
            .is_err());
        let mut stale_claim = stale.clone();
        stale_claim.state = CreatedResourceState::Revoking;
        assert!(store
            .change_created_resource("owner", &stale_claim, Some(&stale))
            .is_err());
        let before = row.clone();
        row.state = CreatedResourceState::Revoking;
        store
            .change_created_resource("owner", &row, Some(&before))
            .unwrap();
        assert!(store
            .change_created_resource("owner", &row, Some(&before))
            .is_err());
        drop(store);
        let reopened = DurableKernelStateStore::open(path).unwrap();
        row.state = CreatedResourceState::RevokeFailed;
        assert_eq!(
            reopened.created_resources("owner", "project").unwrap(),
            vec![row]
        );
    }
}
