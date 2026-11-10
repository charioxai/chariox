//! A borrowed view of host-validated canonical JSON avoids floating point code
//! in the integer-only gate world. Payloads remain exact byte slices.
#[derive(Clone, Copy)]
pub struct Json<'a>(&'a str);
impl<'a> Json<'a> {
    pub fn new(raw: &'a str) -> Self {
        Self(raw.trim())
    }
    pub fn raw(self) -> &'a str {
        self.0
    }
    pub fn integer(self) -> u64 {
        self.0.parse().expect("unsigned integer")
    }
    pub fn text(self) -> &'a str {
        assert!(
            self.0.starts_with('"') && self.0.ends_with('"') && !self.0.contains('\\'),
            "expected plain identifier"
        );
        &self.0[1..self.0.len() - 1]
    }
    pub fn field(self, name: &str) -> Self {
        if !self.0.starts_with('{') {
            return Self("null");
        }
        let mut rest = &self.0[1..self.0.len() - 1];
        while !rest.trim().is_empty() {
            rest = rest.trim_start();
            let key_end = end(rest);
            let key = &rest[..key_end];
            rest = rest[key_end..]
                .trim_start()
                .strip_prefix(':')
                .expect("object colon")
                .trim_start();
            let value_end = end(rest);
            if key.strip_prefix('"').and_then(|s| s.strip_suffix('"')) == Some(name) {
                return Self(&rest[..value_end]);
            }
            rest = rest[value_end..].trim_start();
            if let Some(next) = rest.strip_prefix(',') {
                rest = next;
            } else {
                break;
            }
        }
        Self("null")
    }
    pub fn items(self) -> Vec<Self> {
        assert!(self.0.starts_with('[') && self.0.ends_with(']'), "array");
        let mut rest = &self.0[1..self.0.len() - 1];
        let mut result = Vec::new();
        while !rest.trim().is_empty() {
            rest = rest.trim_start();
            let n = end(rest);
            result.push(Self(&rest[..n]));
            rest = rest[n..].trim_start();
            if let Some(next) = rest.strip_prefix(',') {
                rest = next;
            } else {
                break;
            }
        }
        result
    }
}
fn end(raw: &str) -> usize {
    let bytes = raw.as_bytes();
    let first = bytes[0];
    if first == b'"' {
        let mut i = 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
    } else if first == b'{' || first == b'[' {
        let close = if first == b'{' { b'}' } else { b']' };
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == close {
                return i + 1;
            }
            if matches!(bytes[i], b'"' | b'{' | b'[') {
                i += end(&raw[i..]);
            } else {
                i += 1;
            }
        }
    } else {
        return bytes
            .iter()
            .position(|b| matches!(b, b',' | b'}' | b']') || b.is_ascii_whitespace())
            .unwrap_or(bytes.len());
    }
    panic!("host must supply valid JSON")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_payload_is_exact() {
        let raw = r#"{"data":{"title":"escaped \\\" } []","nested":[{"n":1}]},"phase":"trigger","allowed_entry_nodes":["a","b"]}"#;
        let root = Json::new(raw);
        assert_eq!(
            root.field("data").field("nested").items()[0]
                .field("n")
                .integer(),
            1
        );
        assert_eq!(root.field("phase").text(), "trigger");
        assert_eq!(root.field("allowed_entry_nodes").items()[1].text(), "b");
    }
}
