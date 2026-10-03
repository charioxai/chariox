use super::*;
use rusqlite::Transaction;

/// Compose an authenticated kernel enrollment decision and its operation receipt
/// in one existing writer transaction. This is persistence, not authorization.
/// A savepoint also protects callers that handle an enrollment error and commit
/// other work in their transaction.
///
/// ```compile_fail
/// use chariox_app_runtime::publisher_trust::{enroll_in, TrustDecision};
/// fn autocommit(connection: &rusqlite::Connection, publisher: &chariox_app_package::TrustedPublisher) {
///     enroll_in(connection, "owner", publisher, 0, &TrustDecision { decision_id:"d".into(), authority_ref:"a".into() }, 1).unwrap();
/// }
/// ```
pub fn enroll_in(
    transaction: &Transaction<'_>,
    owner: &str,
    publisher: &TrustedPublisher,
    expected: u64,
    decision: &TrustDecision,
    now_ms: u64,
) -> Result<TrustDecisionReceipt> {
    valid_owner(owner)?;
    valid_identifier(&publisher.publisher_id)?;
    valid_identifier(&publisher.key_id)?;
    valid_text(&decision.decision_id, 128)?;
    valid_text(&decision.authority_ref, 512)?;
    checked_integer(expected)?;
    checked_integer(now_ms)?;
    if publisher.public_key.is_weak() {
        return Err(PublisherTrustError::Invalid);
    }
    transaction.execute_batch("SAVEPOINT app_publisher_enrollment")?;
    let result = store::decide(
        transaction,
        owner,
        &publisher.publisher_id,
        &publisher.key_id,
        Some(publisher.public_key.to_bytes()),
        expected,
        decision,
        now_ms,
    );
    match result {
        Ok(receipt) => {
            transaction.execute_batch("RELEASE app_publisher_enrollment")?;
            Ok(receipt)
        }
        Err(error) => {
            transaction.execute_batch(
                "ROLLBACK TO app_publisher_enrollment; RELEASE app_publisher_enrollment",
            )?;
            Err(error)
        }
    }
}
