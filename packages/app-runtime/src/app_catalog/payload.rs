use super::{CatalogError, Result, MAX_CALL_BYTES, MAX_CALL_NODES};
use serde_json::Value;
use std::io::{self, Write};

pub(super) fn validate(value: &Value) -> Result<()> {
    tree(
        value,
        0,
        &mut Budget {
            nodes: MAX_CALL_NODES,
            bytes: MAX_CALL_BYTES,
        },
    )?;
    serde_json::to_writer(Bounded(0), value).map_err(|_| CatalogError::Limit)
}

struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    fn bytes(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(CatalogError::Limit)?;
        Ok(())
    }
}

fn tree(value: &Value, depth: usize, budget: &mut Budget) -> Result<()> {
    budget.nodes = budget.nodes.checked_sub(1).ok_or(CatalogError::Limit)?;
    budget.bytes(1)?;
    if let Value::String(text) = value {
        budget.bytes(text.len())?;
    }
    if value.is_array() || value.is_object() {
        // Leave two enclosing containers for wire envelope + invocation params.
        if depth >= 62 {
            return Err(CatalogError::Limit);
        }
        match value {
            Value::Array(items) => {
                for value in items {
                    tree(value, depth + 1, budget)?;
                }
            }
            Value::Object(items) => {
                for (key, value) in items {
                    budget.bytes(key.len())?;
                    tree(value, depth + 1, budget)?;
                }
            }
            _ => unreachable!(),
        }
    }
    if let Value::Number(number) = value {
        let number = number.as_f64().ok_or(CatalogError::Invalid)?;
        if !number.is_finite() || (number.fract() == 0.0 && number.abs() > 9_007_199_254_740_991.0)
        {
            return Err(CatalogError::Invalid);
        }
    }
    Ok(())
}

struct Bounded(usize);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_CALL_BYTES - self.0 {
            return Err(io::Error::other("app_catalog_limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
