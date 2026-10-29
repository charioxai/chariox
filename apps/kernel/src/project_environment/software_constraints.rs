//! MP-08 / MP-10 / MP-11: reject proven version contradictions during revision review.
// Unknown syntax stays reviewable; Check reports unsupported constraints without claiming readiness.
type Version = [u32; 4];
#[derive(Clone, Copy)]
struct Range {
    lower: Version,
    lower_inclusive: bool,
    upper: Option<(Version, bool)>,
}
impl Range {
    fn any() -> Self {
        Self {
            lower: [0; 4],
            lower_inclusive: true,
            upper: None,
        }
    }
    fn intersect(self, other: Self) -> Self {
        let (lower, lower_inclusive) = match self.lower.cmp(&other.lower) {
            std::cmp::Ordering::Less => (other.lower, other.lower_inclusive),
            std::cmp::Ordering::Greater => (self.lower, self.lower_inclusive),
            std::cmp::Ordering::Equal => {
                (self.lower, self.lower_inclusive && other.lower_inclusive)
            }
        };
        let upper = match (self.upper, other.upper) {
            (None, upper) | (upper, None) => upper,
            (Some((a, ai)), Some((b, bi))) => Some(match a.cmp(&b) {
                std::cmp::Ordering::Less => (a, ai),
                std::cmp::Ordering::Greater => (b, bi),
                std::cmp::Ordering::Equal => (a, ai && bi),
            }),
        };
        Self {
            lower,
            lower_inclusive,
            upper,
        }
    }
    fn nonempty(self) -> bool {
        // Versions are discrete tuples, so strict adjacent bounds can also be empty.
        let mut first = self.lower;
        if !self.lower_inclusive {
            let Some(index) = first.iter().rposition(|n| *n < u32::MAX) else {
                return false;
            };
            first[index] += 1;
            first[index + 1..].fill(0);
        }
        self.upper
            .is_none_or(|(upper, inclusive)| first < upper || inclusive && first == upper)
    }
}
fn token_range(token: &str) -> Option<Range> {
    if token == "*" {
        return Some(Range::any());
    }
    let op = [">=", "<=", ">", "<", "^", "~", "="]
        .into_iter()
        .find(|op| token.starts_with(op))
        .unwrap_or("");
    let expected = token.strip_prefix(op)?.trim_start_matches('v');
    let wildcard = expected.ends_with(".*") || expected.ends_with(".x");
    let expected = if wildcard {
        &expected[..expected.len() - 2]
    } else {
        expected
    };
    let parts: Vec<u32> = expected
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.is_empty() || parts.len() > 4 || wildcard && parts.len() > 2 {
        return None;
    }
    let mut lower = [0; 4];
    lower[..parts.len()].copy_from_slice(&parts);
    let mut upper = lower;
    let index = match op {
        "^" => parts
            .iter()
            .position(|n| *n != 0)
            .unwrap_or(parts.len() - 1),
        "~" if !wildcard => {
            if parts.len() > 1 {
                1
            } else {
                0
            }
        }
        _ => parts.len() - 1,
    };
    let next = upper[index].checked_add(1).map(|n| {
        upper[index] = n;
        upper[index + 1..].fill(0);
        upper
    });
    let mut range = Range::any();
    match op {
        ">=" => range.lower = lower,
        ">" if wildcard => range.lower = next?,
        ">" => {
            range.lower = lower;
            range.lower_inclusive = false
        }
        "<" => range.upper = Some((lower, false)),
        "<=" if wildcard => range.upper = Some((next?, false)),
        "<=" => range.upper = Some((lower, true)),
        "" | "=" | "^" | "~" => {
            range.lower = lower;
            range.upper = if parts.len() == 4 && matches!(op, "" | "=") {
                Some((lower, true))
            } else {
                next.map(|v| (v, false))
            };
        }
        _ => return None,
    }
    Some(range)
}
fn ranges(constraint: &str) -> Option<Vec<Range>> {
    constraint
        .split("||")
        .map(|part| {
            if part.trim().is_empty() {
                return None;
            }
            part.split_whitespace()
                .try_fold(Range::any(), |range, token| {
                    Some(range.intersect(token_range(token)?))
                })
        })
        .collect()
}
pub(super) fn overlap(a: Option<&str>, b: Option<&str>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return true;
    };
    if a == b {
        return true;
    }
    let (Some(a), Some(b)) = (ranges(a), ranges(b)) else {
        return true;
    };
    a.iter()
        .any(|a| b.iter().any(|b| a.intersect(*b).nonempty()))
}
