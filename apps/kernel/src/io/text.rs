use crate::io::types::{AgentEditOperation, ArtifactEditError, TextRange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextEditPlan {
    pub(crate) range: TextRange,
    pub(crate) old_text: String,
    pub(crate) new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChangeDelta {
    range: TextRange,
    removed_len: usize,
    inserted_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSpan<'a> {
    text: &'a str,
    range: TextRange,
}

pub struct TextDocumentDomain;

impl TextDocumentDomain {
    pub(crate) fn plan_operation(
        base: &str,
        operation: &AgentEditOperation,
    ) -> Result<TextEditPlan, ArtifactEditError> {
        match operation {
            AgentEditOperation::ReplaceText { old_text, new_text } => {
                let start =
                    base.find(old_text)
                        .ok_or_else(|| ArtifactEditError::InvalidOperation {
                            message: "old_text was not found in the base snapshot".to_string(),
                        })?;
                let end = start + old_text.len();
                if base[end..].contains(old_text) {
                    return Err(ArtifactEditError::InvalidOperation {
                        message: "old_text matched more than once; use a range-based edit"
                            .to_string(),
                    });
                }
                Ok(TextEditPlan {
                    range: TextRange::new(start, end),
                    old_text: old_text.clone(),
                    new_text: new_text.clone(),
                })
            }
            AgentEditOperation::ReplaceRange {
                range,
                old_text,
                new_text,
            } => {
                Self::validate_boundary(base, range.start)?;
                Self::validate_boundary(base, range.end)?;
                if range.start > range.end || range.end > base.len() {
                    return Err(ArtifactEditError::InvalidOperation {
                        message: "range is outside the base snapshot".to_string(),
                    });
                }
                if &base[range.start..range.end] != old_text {
                    if !old_text.is_empty() {
                        return Self::plan_operation(
                            base,
                            &AgentEditOperation::ReplaceText {
                                old_text: old_text.clone(),
                                new_text: new_text.clone(),
                            },
                        );
                    }
                    return Err(ArtifactEditError::InvalidOperation {
                        message: "range content does not match old_text in the base snapshot"
                            .to_string(),
                    });
                }
                Ok(TextEditPlan {
                    range: *range,
                    old_text: old_text.clone(),
                    new_text: new_text.clone(),
                })
            }
            AgentEditOperation::WriteArtifact { content } => {
                let new_text =
                    content
                        .as_text()
                        .ok_or_else(|| ArtifactEditError::UnsupportedDomain {
                            domain: crate::io::types::ArtifactDomainKind::TextDocument,
                        })?;
                Ok(TextEditPlan {
                    range: TextRange::new(0, base.len()),
                    old_text: base.to_string(),
                    new_text: new_text.to_string(),
                })
            }
        }
    }

    pub(crate) fn changed_ranges(base: &str, current: &str) -> Vec<TextRange> {
        Self::change_deltas(base, current)
            .into_iter()
            .map(|delta| delta.range)
            .collect()
    }

    pub(crate) fn rebase_plan(
        base: &str,
        current: &str,
        plan: &TextEditPlan,
    ) -> Result<TextEditPlan, ArtifactEditError> {
        if base == current {
            return Ok(plan.clone());
        }
        let deltas = Self::change_deltas(base, current);
        if deltas
            .iter()
            .any(|delta| Self::delta_conflicts_with_range(delta, plan.range))
        {
            return Err(ArtifactEditError::Conflict {
                path: Default::default(),
                base_version: crate::io::types::ArtifactVersion::initial(),
                current_version: crate::io::types::ArtifactVersion::initial(),
                requested_ranges: vec![plan.range],
                changed_ranges: deltas.iter().map(|delta| delta.range).collect(),
                message: "edit overlaps changes made since the base snapshot".to_string(),
            });
        }
        let mut start = plan.range.start;
        let mut end = plan.range.end;
        for delta in deltas {
            if delta.range.end <= plan.range.start {
                let shift = delta.inserted_len as isize - delta.removed_len as isize;
                start = Self::shift_offset(start, shift)?;
                end = Self::shift_offset(end, shift)?;
            }
        }
        Ok(TextEditPlan {
            range: TextRange::new(start, end),
            old_text: plan.old_text.clone(),
            new_text: plan.new_text.clone(),
        })
    }

    pub(crate) fn apply_plan(
        current: &str,
        plan: &TextEditPlan,
    ) -> Result<String, ArtifactEditError> {
        Self::validate_boundary(current, plan.range.start)?;
        Self::validate_boundary(current, plan.range.end)?;
        if plan.range.start > plan.range.end || plan.range.end > current.len() {
            return Err(ArtifactEditError::InvalidOperation {
                message: "rebased range is outside the current artifact".to_string(),
            });
        }
        if &current[plan.range.start..plan.range.end] != plan.old_text {
            return Err(ArtifactEditError::InvalidOperation {
                message: "rebased range content does not match old_text".to_string(),
            });
        }
        let mut output =
            String::with_capacity(current.len() - plan.range.len() + plan.new_text.len());
        output.push_str(&current[..plan.range.start]);
        output.push_str(&plan.new_text);
        output.push_str(&current[plan.range.end..]);
        Ok(output)
    }

    fn change_deltas(base: &str, current: &str) -> Vec<ChangeDelta> {
        if base == current {
            return Vec::new();
        }
        let base_lines = line_spans(base);
        let current_lines = line_spans(current);
        let matches = line_lcs_matches(&base_lines, &current_lines);
        let mut deltas = Vec::new();
        let mut base_index = 0;
        let mut current_index = 0;
        for (matched_base, matched_current) in matches {
            push_delta(
                &mut deltas,
                &base_lines,
                &current_lines,
                base_index,
                matched_base,
                current_index,
                matched_current,
            );
            base_index = matched_base + 1;
            current_index = matched_current + 1;
        }
        push_delta(
            &mut deltas,
            &base_lines,
            &current_lines,
            base_index,
            base_lines.len(),
            current_index,
            current_lines.len(),
        );
        deltas
    }

    fn delta_conflicts_with_range(delta: &ChangeDelta, range: TextRange) -> bool {
        if delta.range.is_empty() {
            return range.start < delta.range.start && delta.range.start < range.end;
        }
        range.overlaps(delta.range)
    }

    fn shift_offset(offset: usize, shift: isize) -> Result<usize, ArtifactEditError> {
        if shift.is_negative() {
            offset.checked_sub(shift.unsigned_abs()).ok_or_else(|| {
                ArtifactEditError::InvalidOperation {
                    message: "rebased range moved before start of artifact".to_string(),
                }
            })
        } else {
            offset
                .checked_add(shift as usize)
                .ok_or_else(|| ArtifactEditError::InvalidOperation {
                    message: "rebased range overflowed artifact bounds".to_string(),
                })
        }
    }

    fn validate_boundary(text: &str, offset: usize) -> Result<(), ArtifactEditError> {
        if offset <= text.len() && text.is_char_boundary(offset) {
            return Ok(());
        }
        Err(ArtifactEditError::InvalidOperation {
            message: "range does not fall on UTF-8 character boundaries".to_string(),
        })
    }
}

fn line_spans(text: &str) -> Vec<LineSpan<'_>> {
    let mut spans = Vec::new();
    let mut start = 0;
    for segment in text.split_inclusive('\n') {
        let end = start + segment.len();
        spans.push(LineSpan {
            text: segment,
            range: TextRange::new(start, end),
        });
        start = end;
    }
    if start < text.len() {
        spans.push(LineSpan {
            text: &text[start..],
            range: TextRange::new(start, text.len()),
        });
    }
    spans
}

fn line_lcs_matches(base: &[LineSpan<'_>], current: &[LineSpan<'_>]) -> Vec<(usize, usize)> {
    let mut lengths = vec![vec![0usize; current.len() + 1]; base.len() + 1];
    for i in (0..base.len()).rev() {
        for j in (0..current.len()).rev() {
            lengths[i][j] = if base[i].text == current[j].text {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let mut matches = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < base.len() && j < current.len() {
        if base[i].text == current[j].text {
            matches.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] > lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matches
}

fn push_delta(
    deltas: &mut Vec<ChangeDelta>,
    base_lines: &[LineSpan<'_>],
    current_lines: &[LineSpan<'_>],
    base_start: usize,
    base_end: usize,
    current_start: usize,
    current_end: usize,
) {
    if base_start == base_end && current_start == current_end {
        return;
    }
    let start = base_lines
        .get(base_start)
        .map(|line| line.range.start)
        .or_else(|| {
            base_lines
                .get(base_start.saturating_sub(1))
                .map(|line| line.range.end)
        })
        .unwrap_or(0);
    let end = if base_start == base_end {
        start
    } else {
        base_lines
            .get(base_end.saturating_sub(1))
            .map(|line| line.range.end)
            .unwrap_or(start)
    };
    let current_start_offset = current_lines
        .get(current_start)
        .map(|line| line.range.start)
        .or_else(|| {
            current_lines
                .get(current_start.saturating_sub(1))
                .map(|line| line.range.end)
        })
        .unwrap_or(0);
    let current_end_offset = if current_start == current_end {
        current_start_offset
    } else {
        current_lines
            .get(current_end.saturating_sub(1))
            .map(|line| line.range.end)
            .unwrap_or(current_start_offset)
    };
    deltas.push(ChangeDelta {
        range: TextRange::new(start, end),
        removed_len: end.saturating_sub(start),
        inserted_len: current_end_offset.saturating_sub(current_start_offset),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_range_with_mismatched_nonempty_old_text_falls_back_to_unique_text() {
        let plan = TextDocumentDomain::plan_operation(
            "one\nTARGET\nthree\n",
            &AgentEditOperation::ReplaceRange {
                range: TextRange::new(0, 0),
                old_text: "TARGET".to_string(),
                new_text: "UPDATED".to_string(),
            },
        )
        .expect("unique old_text should recover from a guessed range");

        assert_eq!(plan.range, TextRange::new(4, 10));
        assert_eq!(
            TextDocumentDomain::apply_plan("one\nTARGET\nthree\n", &plan).unwrap(),
            "one\nUPDATED\nthree\n"
        );
    }

    #[test]
    fn replace_range_with_mismatched_repeated_old_text_still_rejects() {
        let error = TextDocumentDomain::plan_operation(
            "TARGET\nTARGET\n",
            &AgentEditOperation::ReplaceRange {
                range: TextRange::new(0, 0),
                old_text: "TARGET".to_string(),
                new_text: "UPDATED".to_string(),
            },
        )
        .expect_err("ambiguous old_text must not recover from a guessed range");

        assert!(matches!(error, ArtifactEditError::InvalidOperation { .. }));
    }
}
