//! MP-08 / MP-10 / MP-11: instruction discovery exposes bounded metadata, not package manifests.
use super::{catalog::failure, resolver::metadata_context, v2::SkillProjection};
use crate::error::DaemonError;
use serde_json::{json, Value};

fn rendered(revision: u64, rows: &[Value], next: Option<&str>) -> Value {
    json!({"kind":"page","revision":revision,"skills":rows,"next_cursor":next})
}
/// The cursor advances by rows actually emitted, including when a requested page
/// is shortened. This private tool view does not alter human catalog/Details DTOs.
pub(crate) fn page(
    revision: u64,
    skills: &[SkillProjection],
    next: Option<&str>,
    cursor: Option<&str>,
    capacity: Option<u64>,
    percent: f64,
) -> Result<Value, DaemonError> {
    let budget = metadata_context(&[], capacity, percent)?.budget_tokens;
    let offset = match cursor {
        None => 0usize,
        Some(value) => {
            let (version, index) = value
                .split_once(':')
                .ok_or_else(|| failure("invalid skill discovery cursor"))?;
            if version.parse::<u64>().ok() != Some(revision) {
                return Err(failure("catalog changed; restart skill discovery"));
            }
            index
                .parse::<usize>()
                .map_err(|_| failure("invalid skill discovery cursor"))?
        }
    };
    let next_for = |count: usize| -> Option<String> {
        if count < skills.len() {
            Some(format!("{revision}:{}", offset.saturating_add(count)))
        } else {
            next.map(str::to_owned)
        }
    };
    let fits = |rows: &[Value]| {
        rendered(revision, rows, next_for(rows.len()).as_deref())
            .to_string()
            .len() as u64
            <= budget
    };
    let mut rows = Vec::new();
    for row in skills {
        let description = row
            .skill
            .short_description
            .as_ref()
            .unwrap_or(&row.skill.description);
        let mut short = String::new();
        for word in description.split_whitespace() {
            if short.len() + word.len() + usize::from(!short.is_empty()) > 128 {
                break;
            }
            if !short.is_empty() {
                short.push(' ')
            }
            short.push_str(word);
        }
        rows.push(json!({"id":row.skill.id,"name":row.skill.name,"description":short}));
        while !fits(&rows) {
            let description = rows.last_mut().unwrap().get_mut("description").unwrap();
            let text = description.as_str().unwrap();
            if text.is_empty() {
                break;
            }
            *description = Value::String(
                text.rsplit_once(' ')
                    .map_or("", |(head, _)| head)
                    .to_owned(),
            );
        }
        if !fits(&rows) {
            rows.pop();
            break;
        }
    }
    if (!skills.is_empty() && rows.is_empty()) || !fits(&rows) {
        return Err(failure("skill metadata budget is too small for a discovery page; increase skills.metadata_budget_percent or select the skill explicitly"));
    }
    Ok(rendered(revision, &rows, next_for(rows.len()).as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::{resolver::discovery_page, v2::SkillState};
    fn row(index: usize) -> SkillProjection {
        serde_json::from_value(json!({
            "skill":{"id":format!("skill-{index:032x}"),"name":format!("review-{index}"),
                "scope":{"kind":"chariox"},"description":"Quoted \"résumé\" references and reproducible reviews with bundled guidance. ".repeat(120),
                "default_state":"auto","digest":"a".repeat(64),
                "files":(0..100).map(|n|json!({"path":format!("references/{n}.md"),"sha256":"b".repeat(64),"bytes":4096,"executable":false})).collect::<Vec<_>>(),
                "provenance":{"kind":"content","installed_at_ms":1,"updated_at_ms":1},"compatibility_notes":[]},
            "effective_state":"auto","delivery":{"kind":"chariox"}
        })).unwrap()
    }
    #[test]
    fn skill_model_metadata_pages_fit_the_ceiling_without_skipping_rows_or_exposing_files() {
        let mut all = (0..60).map(row).collect::<Vec<_>>();
        all[10].effective_state = SkillState::Manual;
        all[20].effective_state = SkillState::Off;
        let expected = all
            .iter()
            .filter(|row| row.effective_state == SkillState::Auto)
            .map(|row| row.skill.id.clone())
            .collect::<Vec<_>>();
        let mut cursor = None;
        let mut seen = Vec::new();
        let mut pages = 0;
        loop {
            let (entries, next) = discovery_page(&all, 7, cursor.as_deref(), 16).unwrap();
            let legacy = serde_json::to_value(crate::skill::v2::SkillsV2Response::Page {
                revision: 7,
                skills: entries.clone(),
                next_cursor: next.clone(),
            })
            .unwrap();
            assert!(
                legacy.to_string().len() > 2000,
                "the original complete-record response exceeds the metadata ceiling"
            );
            let result = page(
                7,
                &entries,
                next.as_deref(),
                cursor.as_deref(),
                Some(200_000),
                1.0,
            )
            .unwrap();
            assert!(result.to_string().len() <= 2000);
            let rows = result["skills"].as_array().unwrap();
            assert!(!rows.is_empty());
            for row in rows {
                assert_eq!(row.as_object().unwrap().len(), 3);
                for field in ["files", "provenance", "digest", "body"] {
                    assert!(row.get(field).is_none())
                }
                seen.push(row["id"].as_str().unwrap().to_owned());
            }
            cursor = result["next_cursor"].as_str().map(str::to_owned);
            pages += 1;
            if cursor.is_none() {
                break;
            }
            assert!(pages <= 60, "cursor must make progress");
        }
        assert_eq!(seen, expected);
        assert!(pages > 1);
    }
    #[test]
    fn skill_model_metadata_counts_json_escaping_and_shortens_at_word_boundaries() {
        let mut entry = row(0);
        entry.skill.short_description = Some("é \"quoted\" \\ path examples ".repeat(20));
        let result = page(3, &[entry.clone()], None, None, Some(32_768), 1.0).unwrap();
        assert!(result.to_string().len() <= 327);
        let short = result["skills"][0]["description"].as_str().unwrap();
        assert!(!short.is_empty());
        assert!(entry
            .skill
            .short_description
            .as_ref()
            .unwrap()
            .starts_with(short));
        assert_eq!(
            serde_json::from_str::<Value>(&result.to_string()).unwrap(),
            result
        );
        assert!(
            page(3, &[entry], None, None, Some(8192), 0.1).is_err(),
            "never emit a non-progressing undersized page"
        );
        assert!(page(3, &[], None, Some("2:0"), Some(8192), 1.0).is_err());
        assert!(page(3, &[], None, None, None, 1.0).unwrap()["skills"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
