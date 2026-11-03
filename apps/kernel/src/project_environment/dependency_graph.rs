//! MP-08 / MP-10 / MP-11: shared dependency admission and Check projection.
use super::{Requirement, RequirementSpec};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

// Nodes left after topological removal are cycles, their dependents, or
// dependents of absent nodes. Save validates absent IDs before calling this;
// older persisted revisions still fail closed during Check and reads.
pub(super) fn blocked<'a>(rows: impl Iterator<Item = &'a Requirement>) -> BTreeSet<&'a str> {
    let mut remaining = BTreeMap::new();
    let mut dependents = BTreeMap::<&str, Vec<&str>>::new();
    for row in rows {
        let service = match &row.spec {
            RequirementSpec::Services { depends_on, .. } => depends_on.as_slice(),
            _ => &[],
        };
        let dependencies: BTreeSet<_> = row
            .depends_on
            .iter()
            .chain(service)
            .map(String::as_str)
            .collect();
        let id = row.requirement_id.as_str();
        remaining.insert(id, dependencies.len());
        for dependency in dependencies {
            dependents.entry(dependency).or_default().push(id);
        }
    }
    let mut ready: VecDeque<_> = remaining
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    while let Some(id) = ready.pop_front() {
        remaining.remove(id);
        for dependent in dependents.get(id).into_iter().flatten() {
            if let Some(count) = remaining.get_mut(dependent) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(*dependent);
                }
            }
        }
    }
    remaining.into_keys().collect()
}
