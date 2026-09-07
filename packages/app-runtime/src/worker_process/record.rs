use super::WorkerError;

pub(super) const CONTINUE: &[u8; 8] = b"CXAWGO01";

/// Internal native ABI, never deserialized from a terminal or App request.
pub(super) struct LaunchRecord {
    pub generation: String,
    pub installation: String,
    pub release_digest: String,
    pub roots: [String; 4],
    pub bootstrap: String,
    pub nofile: u32,
    pub cpu_seconds: u32,
    pub heap_mib: u32,
    pub v8_threads: u32,
    pub max_file_bytes: u64,
}

impl LaunchRecord {
    pub fn encode(&self) -> Result<Vec<u8>, WorkerError> {
        let generation = self
            .generation
            .parse::<u64>()
            .map_err(|_| WorkerError::Preparation)?;
        if generation == 0
            || generation > i64::MAX as u64
            || generation.to_string() != self.generation
            || self.installation.is_empty()
            || self.installation.len() > 128
            || !self
                .installation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || self.release_digest.len() != 71
            || !self.release_digest.starts_with("sha256:")
            || !self.release_digest[7..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(32..=4096).contains(&self.nofile)
            || !(1..=86400).contains(&self.cpu_seconds)
            || !(16..=65536).contains(&self.heap_mib)
            || !(1..=4).contains(&self.v8_threads)
            || !(1..=1_u64 << 40).contains(&self.max_file_bytes)
            || self.bootstrap.is_empty()
            || self.bootstrap.len() > 262144
            || self.bootstrap.contains('\0')
        {
            return Err(WorkerError::Preparation);
        }
        for (index, root) in self.roots.iter().enumerate() {
            if !root.starts_with('/')
                || root.len() < 2
                || root.len() > 1024
                || root.bytes().any(|b| b.is_ascii_control())
                || root
                    .split('/')
                    .skip(1)
                    .any(|component| matches!(component, "" | "." | ".."))
                || self.roots[..index].iter().any(|other| {
                    root == other
                        || root.starts_with(&format!("{other}/"))
                        || other.starts_with(&format!("{root}/"))
                })
            {
                return Err(WorkerError::Preparation);
            }
        }
        let mut payload = Vec::new();
        for value in [
            self.nofile,
            self.cpu_seconds,
            self.heap_mib,
            self.v8_threads,
        ] {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        payload.extend_from_slice(&self.max_file_bytes.to_be_bytes());
        for value in [
            &self.generation,
            &self.installation,
            &self.release_digest[7..],
            &self.roots[0],
            &self.roots[1],
            &self.roots[2],
            &self.roots[3],
            &self.bootstrap,
        ] {
            string(&mut payload, value);
        }
        if payload.len() > 280000 {
            return Err(WorkerError::Preparation);
        }
        let mut frame = Vec::with_capacity(payload.len() + 12);
        frame.extend_from_slice(b"CXAWL001");
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Exact length and byte comparison also bounds a hostile startup reply.
    pub fn ready(&self) -> Vec<u8> {
        let mut frame = b"CXAWR001".to_vec();
        for value in [
            &self.generation,
            &self.installation,
            &self.release_digest[7..],
        ] {
            string(&mut frame, value);
        }
        frame
    }
}

fn string(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
}
