//! What an agent proposed for the saved Environment and what the owner dismissed: kept apart from the
//! record, so accepting is an ordinary Save and a dismissed proposal is not made again.
use super::detect::{declared_in, detect};
use super::draft::answer_object;
use super::{
    environment_error, environment_variable_name, extension_name, parse_variables,
    project_environment_protected_name, relative_project_path, ExtensionEntry,
    ProjectEnvironmentRecord,
};
use crate::error::DaemonError;
use crate::local::{
    EnvironmentProposal, EnvironmentProposalChange, EnvironmentProposalEvidence,
    EnvironmentProposalField,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DIRECTORY: &str = "project-environment-proposals";
const MAX_PROPOSALS: usize = 20;
const MAX_TEXT: usize = 2048;

static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    proposals: Vec<EnvironmentProposal>,
    dismissed: Vec<String>,
}

pub(crate) struct ProposalStore {
    root: PathBuf,
}

/// The same proposal gets the same id, so a dismissal holds across checks.
fn proposal_id(
    field: EnvironmentProposalField,
    change: EnvironmentProposalChange,
    text: &str,
) -> String {
    format!(
        "{:.16x}",
        Sha256::digest(format!("{field:?}\0{change:?}\0{text}").as_bytes())
    )
}

impl ProposalStore {
    pub(crate) fn new(private_state_root: &Path) -> Self {
        Self {
            root: private_state_root.join(DIRECTORY),
        }
    }

    fn path(&self, project: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.json", Sha256::digest(project.as_bytes())))
    }

    fn load(&self, project: &str) -> Result<Stored, DaemonError> {
        match fs::read(self.path(project)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| environment_error("the saved proposals are not valid")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(_) => Err(environment_error("the saved proposals are unreadable")),
        }
    }

    fn write(&self, project: &str, stored: &Stored) -> Result<(), DaemonError> {
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("the proposals directory is unavailable"))?;
        let temporary = self.root.join(format!(".tmp-{}", rand::random::<u64>()));
        let bytes = serde_json::to_vec(stored)
            .map_err(|_| environment_error("the proposals could not be encoded"))?;
        let result =
            fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, self.path(project)));
        let _ = fs::remove_file(&temporary);
        result.map_err(|_| environment_error("saving the proposals failed"))
    }

    pub(crate) fn list(&self, project: &str) -> Result<Vec<EnvironmentProposal>, DaemonError> {
        Ok(self.load(project)?.proposals)
    }

    /// Keeps the open proposals the check made again, adds the new ones and leaves out the dismissed.
    pub(crate) fn replace(
        &self,
        project: &str,
        proposals: Vec<EnvironmentProposal>,
    ) -> Result<(), DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stored = self.load(project)?;
        stored.proposals = proposals
            .into_iter()
            .filter(|proposal| !stored.dismissed.contains(&proposal.id))
            .take(MAX_PROPOSALS)
            .collect();
        self.write(project, &stored)
    }

    /// Removes one proposal, remembering the id when it was dismissed; the proposal itself is returned.
    pub(crate) fn resolve(
        &self,
        project: &str,
        proposal_id: &str,
        dismiss: bool,
    ) -> Result<Option<EnvironmentProposal>, DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stored = self.load(project)?;
        let Some(index) = stored.proposals.iter().position(|p| p.id == proposal_id) else {
            return Ok(None);
        };
        let proposal = stored.proposals.remove(index);
        if dismiss {
            stored.dismissed.push(proposal.id.clone());
        }
        self.write(project, &stored)?;
        Ok(Some(proposal))
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// A proposal the saved record can take: an addition that is not there yet, a removal of what is.
fn applicable(record: &ProjectEnvironmentRecord, proposal: &EnvironmentProposal) -> bool {
    let text = proposal.text.as_str();
    let add = proposal.change == EnvironmentProposalChange::Add;
    if text.is_empty() || text.len() > MAX_TEXT || text.contains('\0') {
        return false;
    }
    match proposal.field {
        EnvironmentProposalField::Setup => {
            lines(&record.setup_script).contains(&text) != add && !text.contains('\n')
        }
        EnvironmentProposalField::Variables => {
            let valid = parse_variables(text).is_ok_and(|parsed| parsed.len() == 1);
            valid && !text.contains('\n') && lines(&record.variables).contains(&text) != add
        }
        EnvironmentProposalField::Secret => {
            record.secrets.iter().any(|name| name == text) != add
                && (!add
                    || (environment_variable_name(text)
                        && !project_environment_protected_name(text)
                        && !parse_variables(&record.variables)
                            .unwrap_or_default()
                            .iter()
                            .any(|(name, _)| name == text)))
        }
        EnvironmentProposalField::File => {
            record.files.iter().any(|path| path == text) != add
                && (!add || relative_project_path(text).is_ok())
        }
        EnvironmentProposalField::Mcp | EnvironmentProposalField::Skill => {
            let entries = extensions(record, proposal.field);
            entries.iter().any(|entry| entry.name == text) != add
                && (!add
                    || (extension_name(text)
                        && relative_project_path(&proposal.evidence.path).is_ok()))
        }
    }
}

fn extensions(
    record: &ProjectEnvironmentRecord,
    field: EnvironmentProposalField,
) -> &Vec<ExtensionEntry> {
    if field == EnvironmentProposalField::Mcp {
        &record.mcps
    } else {
        &record.skills
    }
}

/// What the Project's files say against the saved record, with no model: MCPs, skills and variable names
/// the files declare that the record lacks, and entries whose declaring file no longer declares them.
pub(crate) fn deterministic_proposals(
    checkout: &Path,
    record: &ProjectEnvironmentRecord,
) -> Vec<EnvironmentProposal> {
    let detected = detect(checkout);
    let mut proposals = Vec::new();
    let mut propose = |field, change, text: &str, path: &str, excerpt: &str| {
        let proposal = EnvironmentProposal {
            id: proposal_id(field, change, text),
            field,
            change,
            text: text.to_owned(),
            evidence: EnvironmentProposalEvidence {
                path: path.to_owned(),
                excerpt: excerpt.to_owned(),
            },
        };
        if applicable(record, &proposal) {
            proposals.push(proposal);
        }
    };
    for (field, found) in [
        (EnvironmentProposalField::Mcp, &detected.mcps),
        (EnvironmentProposalField::Skill, &detected.skills),
    ] {
        for entry in found {
            if let Some(path) = entry.file_path() {
                propose(
                    field,
                    EnvironmentProposalChange::Add,
                    &entry.name,
                    path,
                    &entry.name,
                );
            }
        }
        // A registration is no project file: this check has nothing to compare it with.
        for entry in extensions(record, field) {
            let Some(path) = entry.file_path() else {
                continue;
            };
            if declared_in(checkout, path).contains(&entry.name) {
                continue;
            }
            let excerpt = if checkout.join(path).is_file() {
                format!("not in {path}")
            } else {
                "file missing".to_owned()
            };
            propose(
                field,
                EnvironmentProposalChange::Remove,
                &entry.name,
                path,
                &excerpt,
            );
        }
    }
    let known = parse_variables(&record.variables).unwrap_or_default();
    for (name, source) in &detected.variables {
        if !known.iter().any(|(variable, _)| variable == name) {
            propose(
                EnvironmentProposalField::Secret,
                EnvironmentProposalChange::Add,
                name,
                source,
                name,
            );
        }
    }
    proposals
}

/// The proposals in the agent's answer that the record can take, as a check would store them.
pub(crate) fn parse_proposals(
    output: &str,
    record: &ProjectEnvironmentRecord,
) -> Result<Vec<EnvironmentProposal>, String> {
    let value =
        answer_object(output, "proposals").ok_or("The agent did not answer with proposals")?;
    let listed = value
        .get("proposals")
        .and_then(|v| v.as_array())
        .ok_or("The agent did not answer with proposals")?;
    let mut proposals: Vec<EnvironmentProposal> = Vec::new();
    for item in listed {
        let text = |key: &str| item.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        let field = match text("field").as_deref() {
            Some("setup") => EnvironmentProposalField::Setup,
            Some("variables") => EnvironmentProposalField::Variables,
            Some("secret") => EnvironmentProposalField::Secret,
            Some("file") => EnvironmentProposalField::File,
            _ => continue,
        };
        let change = match text("change").as_deref() {
            Some("add") => EnvironmentProposalChange::Add,
            Some("remove") => EnvironmentProposalChange::Remove,
            _ => continue,
        };
        let Some(line) = text("text") else { continue };
        let line = line.trim().to_owned();
        let evidence = item.get("evidence");
        let evidence_text = |key: &str| {
            evidence
                .and_then(|e| e.get(key))
                .and_then(|v| v.as_str())
                .map(|v| v.chars().take(200).collect::<String>())
        };
        let (Some(path), Some(excerpt)) = (evidence_text("path"), evidence_text("excerpt")) else {
            continue;
        };
        let proposal = EnvironmentProposal {
            id: proposal_id(field, change, &line),
            field,
            change,
            text: line,
            evidence: EnvironmentProposalEvidence { path, excerpt },
        };
        if applicable(record, &proposal) && !proposals.iter().any(|p| p.id == proposal.id) {
            proposals.push(proposal);
        }
    }
    Ok(proposals)
}

/// The fields after one proposal: a line appended or removed, a name or path added or removed.
pub(crate) fn apply(
    record: &ProjectEnvironmentRecord,
    proposal: &EnvironmentProposal,
) -> Result<ProjectEnvironmentRecord, DaemonError> {
    if !applicable(record, proposal) {
        return Err(environment_error(
            "the Environment changed since this was proposed",
        ));
    }
    let mut next = record.clone();
    let text = proposal.text.as_str();
    let add = proposal.change == EnvironmentProposalChange::Add;
    let edit = |block: &str| {
        if add {
            let mut kept = block.trim_end().to_owned();
            if !kept.is_empty() {
                kept.push('\n');
            }
            kept.push_str(text);
            kept
        } else {
            let mut removed = false;
            block
                .lines()
                .filter(|line| {
                    let hit = !removed && *line == text;
                    removed |= hit;
                    !hit
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    match proposal.field {
        EnvironmentProposalField::Setup => next.setup_script = edit(&record.setup_script),
        EnvironmentProposalField::Variables => next.variables = edit(&record.variables),
        EnvironmentProposalField::Secret => {
            if add {
                next.secrets.push(text.to_owned());
            } else {
                next.secrets.retain(|name| name != text);
            }
        }
        EnvironmentProposalField::File => {
            if add {
                next.files.push(text.to_owned());
            } else {
                next.files.retain(|path| path != text);
            }
        }
        EnvironmentProposalField::Mcp | EnvironmentProposalField::Skill => {
            let entries = if proposal.field == EnvironmentProposalField::Mcp {
                &mut next.mcps
            } else {
                &mut next.skills
            };
            if add {
                entries.push(ExtensionEntry::file(text, &proposal.evidence.path));
            } else {
                entries.retain(|entry| entry.name != text);
            }
        }
    }
    next.validate().map_err(environment_error)?;
    Ok(next)
}

/// The prompt of a check: the Project's facts, what the Environment says now, and the answer's shape.
pub(crate) fn check_prompt(facts: &str, record: &ProjectEnvironmentRecord) -> String {
    format!(
        "Compare the Project below with its saved development Environment and propose what is missing or no longer needed. Answer with one JSON object and nothing else, in this shape:\n\
{{\"proposals\": [{{\"field\": \"setup|variables|secret|file\", \"change\": \"add|remove\", \"text\": \"one setup line, one KEY=value line, a secret name or a file path\", \"evidence\": {{\"path\": \"file that shows it\", \"excerpt\": \"one line from it\"}}}}]}}\n\
- Every proposal needs evidence from the metadata: the file and one line of it. For example a step the README or AGENTS.md asks for that the saved Setup lacks, or a credential name in .env.example that is not among the saved Secrets.\n\
- Setup and variables take single lines. Secrets are names of credentials only. Files must come from the list of ignored files.\n\
- Propose nothing that the saved Environment already has. Answer with an empty list only when the Environment really covers the metadata.\n\n\
=== Saved Environment ===\nSetup script:\n{}\n\nVariables:\n{}\n\nSecret names: {}\nFiles outside Git: {}\n\n\
=== Project metadata ===\n{facts}",
        record.setup_script,
        record.variables,
        record.secrets.join(", "),
        record.files.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with(files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chariox-env-check-{:032x}", rand::random::<u128>()));
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        dir
    }

    #[test]
    fn a_check_without_a_model_proposes_what_the_files_declare_and_drops_what_they_no_longer_do() {
        let dir = project_with(&[
            (".mcp.json", r#"{"mcpServers":{"github":{},"notion":{}}}"#),
            (".env.example", "STRIPE_SECRET_KEY=\nLOG_LEVEL=info\n"),
        ]);
        let mut saved = record();
        saved.mcps = vec![ExtensionEntry::file("github", ".mcp.json")];
        saved.variables = "LOG_LEVEL=debug".into();
        saved.secrets.clear();
        let found = deterministic_proposals(&dir, &saved);
        let shown: Vec<String> = found
            .iter()
            .map(|p| {
                format!(
                    "{:?} {:?} {} @{} ({})",
                    p.field, p.change, p.text, p.evidence.path, p.evidence.excerpt
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                "Mcp Add notion @.mcp.json (notion)".to_owned(),
                "Secret Add STRIPE_SECRET_KEY @.env.example (STRIPE_SECRET_KEY)".to_owned(),
            ],
            "github is already saved and LOG_LEVEL is already a Variable"
        );
        std::fs::remove_file(dir.join(".mcp.json")).unwrap();
        let gone = deterministic_proposals(&dir, &saved);
        let first = gone
            .iter()
            .find(|p| p.field == EnvironmentProposalField::Mcp)
            .unwrap();
        assert_eq!(
            (
                first.change,
                first.text.as_str(),
                first.evidence.path.as_str(),
                first.evidence.excerpt.as_str()
            ),
            (
                EnvironmentProposalChange::Remove,
                "github",
                ".mcp.json",
                "file missing"
            )
        );
        let other = project_with(&[(".mcp.json", r#"{"mcpServers":{"linear":{}}}"#)]);
        let still = deterministic_proposals(&other, &saved);
        let removed = still
            .iter()
            .find(|p| p.change == EnvironmentProposalChange::Remove)
            .unwrap();
        assert_eq!(
            removed.evidence.excerpt, "not in .mcp.json",
            "the file is there but no longer names it"
        );
        let next = apply(&saved, removed).unwrap();
        assert!(next.mcps.is_empty());
        let _ = std::fs::remove_dir_all(other);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn record() -> ProjectEnvironmentRecord {
        ProjectEnvironmentRecord {
            schema: super::super::PROJECT_ENVIRONMENT_SCHEMA,
            revision: 3,
            setup_script: "pnpm install\nmise install".into(),
            variables: "A=1".into(),
            secrets: vec!["DB".into()],
            files: vec![".env".into()],
            ..Default::default()
        }
    }

    const ANSWER: &str = r#"Sure: {"proposals":[
      {"field":"setup","change":"add","text":"pnpm --filter billing prisma generate","evidence":{"path":"package.json","excerpt":"prisma"}},
      {"field":"secret","change":"add","text":"STRIPE_SECRET_KEY","evidence":{"path":"lib/billing.ts","excerpt":"process.env.STRIPE_SECRET_KEY"}},
      {"field":"secret","change":"add","text":"DB","evidence":{"path":"x","excerpt":"already there"}},
      {"field":"file","change":"add","text":"../escape","evidence":{"path":"x","excerpt":"y"}},
      {"field":"setup","change":"remove","text":"mise install","evidence":{"path":"mise.toml","excerpt":"gone"}},
      {"field":"variables","change":"add","text":"NOT A LINE","evidence":{"path":"x","excerpt":"y"}},
      {"field":"file","change":"add","text":"config/local.json","evidence":{}}
    ]}"#;

    #[test]
    fn only_what_the_record_can_take_and_has_evidence_for_is_proposed() {
        let proposals = parse_proposals(ANSWER, &record()).unwrap();
        let described: Vec<_> = proposals
            .iter()
            .map(|p| (p.field, p.change, p.text.as_str()))
            .collect();
        assert_eq!(
            described,
            vec![
                (
                    EnvironmentProposalField::Setup,
                    EnvironmentProposalChange::Add,
                    "pnpm --filter billing prisma generate"
                ),
                (
                    EnvironmentProposalField::Secret,
                    EnvironmentProposalChange::Add,
                    "STRIPE_SECRET_KEY"
                ),
                (
                    EnvironmentProposalField::Setup,
                    EnvironmentProposalChange::Remove,
                    "mise install"
                ),
            ]
        );
        assert_eq!(proposals[1].evidence.path, "lib/billing.ts");
        assert!(parse_proposals("no json", &record()).is_err());
    }

    #[test]
    fn accepting_changes_one_line_or_name_and_a_stale_proposal_is_refused() {
        let proposals = parse_proposals(ANSWER, &record()).unwrap();
        let first = apply(&record(), &proposals[0]).unwrap();
        assert_eq!(
            first.setup_script,
            "pnpm install\nmise install\npnpm --filter billing prisma generate"
        );
        let secret = apply(&record(), &proposals[1]).unwrap();
        assert_eq!(
            secret.secrets,
            vec!["DB".to_string(), "STRIPE_SECRET_KEY".into()]
        );
        let removed = apply(&record(), &proposals[2]).unwrap();
        assert_eq!(removed.setup_script, "pnpm install");
        assert!(
            apply(&removed, &proposals[2]).is_err(),
            "the line is gone already"
        );
        assert!(
            apply(&secret, &proposals[1]).is_err(),
            "the name is there already"
        );
    }

    #[test]
    fn a_dismissed_proposal_is_not_stored_again_and_accepting_forgets_it() {
        let root = std::env::temp_dir().join(format!(
            "chariox-env-proposals-{:032x}",
            rand::random::<u128>()
        ));
        let store = ProposalStore::new(&root);
        let proposals = parse_proposals(ANSWER, &record()).unwrap();
        store.replace("p", proposals.clone()).unwrap();
        assert_eq!(store.list("p").unwrap().len(), 3);
        let dismissed = store.resolve("p", &proposals[1].id, true).unwrap().unwrap();
        assert_eq!(dismissed.text, "STRIPE_SECRET_KEY");
        assert!(store
            .resolve("p", &proposals[1].id, true)
            .unwrap()
            .is_none());
        store.replace("p", proposals.clone()).unwrap();
        let ids: Vec<_> = store.list("p").unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![proposals[0].id.clone(), proposals[2].id.clone()]);
        store.resolve("p", &proposals[0].id, false).unwrap();
        store.replace("p", proposals).unwrap();
        assert_eq!(
            store.list("p").unwrap().len(),
            2,
            "a dismissed one stays out and the others come back"
        );
        let _ = fs::remove_dir_all(root);
    }
}
