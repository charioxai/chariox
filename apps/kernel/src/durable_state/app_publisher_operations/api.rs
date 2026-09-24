use super::*;
use rusqlite::params;

impl DurableKernelStateStore {
    fn publisher_operation(&self, command: Command) -> Result<Reply> {
        let (response, receive) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppPublisherOperation(Box::new(
                PublisherOperationRequest { command, response },
            )))
            .map_err(|_| PublisherOperationError::Storage)?;
        receive
            .recv()
            .map_err(|_| PublisherOperationError::CommitUnknown)?
    }
    pub(crate) fn begin_publisher_enrollment(
        &self,
        owner: &str,
        request: &str,
        input: PublisherEnrollmentInput,
        budget: AppOperationBudget,
    ) -> Result<PublisherOperation> {
        operation(self.publisher_operation(Command::Begin {
            owner: owner.into(),
            request: request.into(),
            input,
            budget,
        })?)
    }
    pub(crate) fn arm_publisher_enrollment(
        &self,
        owner: &str,
        request: &str,
        interaction: &str,
        original_deadline: Instant,
        budget: AppOperationBudget,
    ) -> Result<PublisherReview> {
        match self.publisher_operation(Command::Arm {
            owner: owner.into(),
            request: request.into(),
            interaction: interaction.into(),
            deadline: original_deadline,
            budget,
        })? {
            Reply::Review(review) => Ok(review),
            _ => Err(PublisherOperationError::Storage),
        }
    }
    pub(crate) fn decide_publisher_enrollment(
        &self,
        challenge: Arc<PublisherApprovalChallenge>,
        accepted: bool,
        budget: AppOperationBudget,
    ) -> Result<PublisherOperation> {
        operation(self.publisher_operation(Command::Decide {
            challenge,
            accepted,
            budget,
        })?)
    }
    pub(crate) fn cancel_publisher_enrollment(
        &self,
        owner: &str,
        request: &str,
        budget: AppOperationBudget,
    ) -> Result<PublisherOperation> {
        operation(self.publisher_operation(Command::Cancel {
            owner: owner.into(),
            request: request.into(),
            budget,
        })?)
    }
    pub(crate) fn publisher_enrollment_status(
        &self,
        owner: &str,
        request: &str,
        budget: AppOperationBudget,
    ) -> Result<PublisherOperation> {
        operation(self.publisher_operation(Command::Status {
            owner: owner.into(),
            request: request.into(),
            budget,
        })?)
    }
    /// Discovery only. The retained recovery owner repeats current state on the
    /// writer and arms a fresh nonce; an old pending row does not grant consent.
    pub(crate) fn pending_publisher_enrollments(
        &self,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<(String, String)>> {
        self.require_writer_healthy()
            .map_err(|_| PublisherOperationError::Storage)?;
        let connection = self
            .lock_connection("durable_state.pending_publisher_enrollments")
            .map_err(|_| PublisherOperationError::Storage)?;
        let mut statement=store::sql(connection.prepare("SELECT owner_id,request_id FROM app_publisher_operations WHERE phase='pending' AND (?1 IS NULL OR (owner_id,request_id)>(?1,?2)) ORDER BY owner_id,request_id LIMIT 8"))?;
        let rows = store::sql(
            statement.query_map(params![after.map(|v| v.0), after.map(|v| v.1)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            }),
        )?;
        let values = store::sql(rows.collect())?;
        self.require_writer_healthy()
            .map_err(|_| PublisherOperationError::Storage)?;
        Ok(values)
    }
}
fn operation(reply: Reply) -> Result<PublisherOperation> {
    match reply {
        Reply::Operation(value) => Ok(value),
        _ => Err(PublisherOperationError::Storage),
    }
}
