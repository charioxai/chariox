#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInputRecord {
    pub session_id: String,
    pub provider_run_id: String,
    pub source_attachment_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutputRecord {
    pub session_id: String,
    pub provider_run_id: String,
    pub recipient_attachment_ids: Vec<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: Vec<TerminalOutputRecord>,
}

impl TerminalStreamService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) {
        self.input_records.push(TerminalInputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            source_attachment_id: source_attachment_id.to_string(),
            bytes: bytes.to_vec(),
        });
    }

    pub fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
        };

        self.output_records.push(record.clone());
        record
    }

    pub fn input_records(&self) -> &[TerminalInputRecord] {
        &self.input_records
    }

    pub fn output_records(&self) -> &[TerminalOutputRecord] {
        &self.output_records
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalStreamService;

    #[test]
    fn records_terminal_input_and_fans_out_output() {
        let mut terminal = TerminalStreamService::new();

        terminal.record_input("session-1", "provider-run-1", "attachment-1", b"ls\n");
        let output = terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"listing\n",
        );

        assert_eq!(terminal.input_records().len(), 1);
        assert_eq!(terminal.output_records().len(), 1);
        assert_eq!(output.recipient_attachment_ids.len(), 2);
    }
}
