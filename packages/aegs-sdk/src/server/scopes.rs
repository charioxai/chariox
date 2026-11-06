//! Vets subscription claims against what their connection gives access to.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chariox_event_protocol::{
    AegsProviderResourceQuery, AegsRefusedSubscription, AegsSubscriptionRefusal,
};

use super::AegsServer;
use crate::SubscriptionClaim;

/// A provider lists the resources of a connection page by page and may rate-limit that; one
/// listing serves every claim of the connection for this long.
const SCOPE_LISTING_TTL: Duration = Duration::from_secs(5 * 60);
const RESOURCE_PAGE_LIMIT: u32 = 100;
const MAX_RESOURCE_PAGES: usize = 200;

type Listings = HashMap<(String, Option<i64>), (Instant, Arc<HashSet<String>>)>;

/// Scope listings per connection and authorization generation, so a reauthorized connection is
/// listed again.
#[derive(Clone, Default)]
pub(super) struct ScopeCache(Arc<Mutex<Listings>>);

impl AegsServer {
    /// Splits `claims` into those the connection covers and those refused. An inactive claim
    /// only withdraws a subscription and is never refused. `Err` means the provider could not
    /// list the connection's resources: the caller keeps what is stored and the kernel retries.
    pub(super) async fn vet_subscription_claims(
        &self,
        claims: &[SubscriptionClaim],
    ) -> Result<(Vec<SubscriptionClaim>, Vec<AegsRefusedSubscription>), String> {
        let mut kept = Vec::with_capacity(claims.len());
        let mut refused = Vec::new();
        for claim in claims {
            let verdict = if claim.active {
                self.scope_refusal(claim).await?
            } else {
                None
            };
            match verdict {
                None => kept.push(claim.clone()),
                Some(reason) => refused.push(AegsRefusedSubscription {
                    binding_id: claim.binding_id.clone(),
                    reason,
                }),
            }
        }
        Ok((kept, refused))
    }

    async fn scope_refusal(
        &self,
        claim: &SubscriptionClaim,
    ) -> Result<Option<AegsSubscriptionRefusal>, String> {
        let Some(connection) = self.store.connection(&claim.connection_id)? else {
            return Ok(Some(AegsSubscriptionRefusal::ConnectionNotFound));
        };
        if connection.status != "ready" {
            return Ok(Some(AegsSubscriptionRefusal::ConnectionNotReady));
        }
        let generation = self.store.connection_generation(&claim.connection_id)?;
        let scopes = self
            .connection_scopes(&claim.connection_id, &connection.owner_id, generation)
            .await?;
        Ok((!scopes.contains(&claim.connection_scope))
            .then_some(AegsSubscriptionRefusal::ScopeNotInConnection))
    }

    async fn connection_scopes(
        &self,
        connection_id: &str,
        owner_id: &str,
        generation: Option<i64>,
    ) -> Result<Arc<HashSet<String>>, String> {
        let key = (connection_id.to_string(), generation);
        if let Some((listed_at, scopes)) = self
            .scope_cache
            .0
            .lock()
            .map_err(|_| "scope cache lock was poisoned".to_string())?
            .get(&key)
        {
            if listed_at.elapsed() < SCOPE_LISTING_TTL {
                return Ok(Arc::clone(scopes));
            }
        }
        let provider = Arc::clone(&self.provider);
        let (generator_id, owner_id, connection_id) = (
            self.producer_id.clone(),
            owner_id.to_string(),
            connection_id.to_string(),
        );
        let scopes = tokio::task::spawn_blocking(move || {
            let mut scopes = HashSet::new();
            let mut cursor = None;
            for _ in 0..MAX_RESOURCE_PAGES {
                let page = provider.query_resources(&AegsProviderResourceQuery {
                    generator_id: generator_id.clone(),
                    owner_id: owner_id.clone(),
                    connection_id: connection_id.clone(),
                    query: None,
                    cursor: cursor.take(),
                    limit: RESOURCE_PAGE_LIMIT,
                })?;
                scopes.extend(page.resources.into_iter().map(|r| r.connection_scope));
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => return Ok(scopes),
                }
            }
            Err("the connection lists more resources than the AEGS checks".to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        let scopes = Arc::new(scopes);
        self.scope_cache
            .0
            .lock()
            .map_err(|_| "scope cache lock was poisoned".to_string())?
            .insert(key, (Instant::now(), Arc::clone(&scopes)));
        Ok(scopes)
    }
}
