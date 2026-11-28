//! Row 569: what a `/sudo` prompt grants about the account's machine limits.
//!
//! The owner types `/sudo raise limits to 4 machines`; the kernel reads those words once, when
//! the prompt is submitted, into this structured grant and keeps the structure on the sudo
//! window. Authority never re-reads text. The grammar is strict: the whole prompt is the grant,
//! so a look-alike or a longer sentence grants nothing; the owner must submit the exact named prompt.
//!
//! The parsed grant lives on the window (`KernelSudoTurn.limit_grant`, kernel memory only), and
//! `machine_acquisition::limits` reads it when an agent raises a limit.

use crate::local::LimitScope;

/// The raise the owner named. A field the prompt does not name is not granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitGrant {
    pub scope: LimitScope,
    /// The exact target class list, never a wildcard or implicit grant.
    pub classes: Option<Vec<String>>,
    /// "Machines at once" may be raised to this many, and no more.
    pub machines_at_once: Option<u32>,
    /// "Spend per day" may be raised to this amount (millionths of `currency`), and no more.
    pub spend_per_day: Option<(u64, String)>,
}

/// A raise an agent asks for: only the fields that go above the current value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestedRaise {
    pub machines_at_once: Option<u32>,
    pub spend_per_day_micros: Option<u64>,
}

const MAX_MACHINES: u32 = 100;

impl LimitGrant {
    /// The grant a `/sudo` prompt names, or `None` when the prompt is anything else.
    pub(crate) fn parse(prompt: &str) -> Option<Self> {
        fn word(input: &str) -> (&str, &str) {
            let input = input.trim_start();
            input.find(char::is_whitespace).map_or((input, ""), |at| {
                (&input[..at], input[at..].trim_start())
            })
        }
        let prompt = prompt.trim().trim_end_matches('.');
        let (raise, rest) = word(prompt);
        if !raise.eq_ignore_ascii_case("raise") { return None; }
        let (scope_or_limits, rest) = word(rest);
        let (scope, rest) = if scope_or_limits.eq_ignore_ascii_case("limits") {
            (LimitScope::Account, rest)
        } else {
            let scope = if scope_or_limits.eq_ignore_ascii_case("account") {
                LimitScope::Account
            } else if scope_or_limits.eq_ignore_ascii_case("no-ask") {
                LimitScope::NoAsk
            } else { return None; };
            let (limits, rest) = word(rest);
            if !limits.eq_ignore_ascii_case("limits") { return None; }
            (scope, rest)
        };
        let (to, rest) = word(rest);
        if !to.eq_ignore_ascii_case("to") { return None; }
        let lowered = rest.to_ascii_lowercase();
        let (numeric, raw_classes) = if lowered.starts_with("classes ") {
            ("", Some(&rest[8..]))
        } else if let Some(at) = lowered.find(" and classes ") {
            (&rest[..at], Some(&rest[at + 13..]))
        } else { (rest, None) };
        let classes = if let Some(raw) = raw_classes {
            if scope != LimitScope::NoAsk { return None; }
            let classes: Vec<String> = serde_json::from_str(raw).ok()?;
            if classes.iter().any(|name| name.is_empty() || name.chars().any(char::is_control))
                || classes.iter().collect::<std::collections::BTreeSet<_>>().len() != classes.len() {
                return None;
            }
            Some(classes)
        } else { None };
        let mut grant = LimitGrant { scope, classes, machines_at_once: None, spend_per_day: None };
        if !numeric.trim().is_empty() {
            let lowered = numeric.to_lowercase();
            let words: Vec<&str> = lowered.split_whitespace().collect();
            for clause in words.split(|word| *word == "and") {
                match clause {
                    [count, "machine" | "machines"] => {
                        if grant.machines_at_once.is_some() { return None; }
                        let count: u32 = count.parse().ok().filter(|n| (1..=MAX_MACHINES).contains(n))?;
                        grant.machines_at_once = Some(count);
                    }
                    [amount, currency, "a" | "per", "day"] => {
                        if grant.spend_per_day.is_some() { return None; }
                        let micros = spend_micros(amount)?;
                        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) { return None; }
                        grant.spend_per_day = Some((micros, currency.to_uppercase()));
                    }
                    _ => return None,
                }
            }
        }
        (grant.machines_at_once.is_some() || grant.spend_per_day.is_some() || grant.classes.is_some()).then_some(grant)
    }

    /// Whether the grant covers every field of `raise`, in the account's currency.
    /// A field the raise leaves alone needs no cover; an empty raise is covered by any grant.
    pub(crate) fn covers(&self, raise: &RequestedRaise, account_currency: &str) -> bool {
        let machines = raise
            .machines_at_once
            .is_none_or(|wanted| self.machines_at_once.is_some_and(|named| wanted <= named));
        let spend = raise.spend_per_day_micros.is_none_or(|wanted| {
            self.spend_per_day
                .as_ref()
                .is_some_and(|(named, currency)| {
                    wanted <= *named && currency.eq_ignore_ascii_case(account_currency)
                })
        });
        machines && spend
    }

    pub(crate) fn covers_change(&self, scope: LimitScope, raise: &RequestedRaise, currency: &str, classes: Option<&[String]>) -> bool {
        self.scope == scope && self.covers(raise, currency) && classes.is_none_or(|wanted| {
            self.classes.as_ref().is_some_and(|named| {
                wanted.iter().collect::<std::collections::BTreeSet<_>>() == named.iter().collect::<std::collections::BTreeSet<_>>()
            })
        })
    }

    /// The typed owner step for this exact request; nothing representable is rounded or widened.
    pub(crate) fn prompt_for(scope: LimitScope, raise: &RequestedRaise, currency: &str, classes: Option<&[String]>) -> Option<String> {
        let mut clauses = Vec::new();
        if let Some(count) = raise.machines_at_once { clauses.push(format!("{count} machines")); }
        if let Some(micros) = raise.spend_per_day_micros {
            if micros % 10_000 != 0 { return None; }
            clauses.push(format!("{}.{:02} {currency} a day", micros / 1_000_000, (micros % 1_000_000) / 10_000));
        }
        if let Some(classes) = classes { clauses.push(format!("classes {}", serde_json::to_string(classes).ok()?)); }
        let scope_name = match scope { LimitScope::Account => "account", LimitScope::NoAsk => "no-ask" };
        let prompt = format!("raise {scope_name} limits to {}", clauses.join(" and "));
        Self::parse(&prompt).filter(|grant| grant.covers_change(scope, raise, currency, classes)).map(|_| prompt)
    }
}

/// "10", "10.5", "0.07" -> millionths; nothing else (no sign, no third decimal, no separators).
fn spend_micros(text: &str) -> Option<u64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || whole.len() > 9
        || fraction.len() > 2
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let cents: u64 = format!("{fraction:0<2}").parse().ok()?;
    Some(whole.parse::<u64>().ok()? * 1_000_000 + cents * 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machines(n: u32) -> LimitGrant {
        LimitGrant {
            scope: LimitScope::Account,
            classes: None,
            machines_at_once: Some(n),
            spend_per_day: None,
        }
    }

    #[test]
    fn the_named_machines_are_the_grant() {
        assert_eq!(
            LimitGrant::parse("raise limits to 4 machines"),
            Some(machines(4))
        );
        assert_eq!(
            LimitGrant::parse("  Raise limits to 4 Machines. "),
            Some(machines(4))
        );
        assert_eq!(
            LimitGrant::parse("raise limits to 1 machine"),
            Some(machines(1))
        );
    }

    #[test]
    fn spend_and_both_in_either_order() {
        let spend = |micros, cur: &str| Some((micros, cur.to_owned()));
        assert_eq!(
            LimitGrant::parse("raise limits to 15 eur a day"),
            Some(LimitGrant {
                scope: LimitScope::Account,
                classes: None,
                machines_at_once: None,
                spend_per_day: spend(15_000_000, "EUR")
            })
        );
        assert_eq!(
            LimitGrant::parse("raise limits to 10.5 USD per day"),
            Some(LimitGrant {
                scope: LimitScope::Account,
                classes: None,
                machines_at_once: None,
                spend_per_day: spend(10_500_000, "USD")
            })
        );
        let both = LimitGrant {
            scope: LimitScope::Account,
            classes: None,
            machines_at_once: Some(4),
            spend_per_day: spend(15_000_000, "EUR"),
        };
        assert_eq!(
            LimitGrant::parse("raise limits to 4 machines and 15 EUR a day"),
            Some(both.clone())
        );
        assert_eq!(
            LimitGrant::parse("raise limits to 15 EUR a day and 4 machines"),
            Some(both)
        );
    }

    #[test]
    fn look_alikes_and_longer_sentences_grant_nothing() {
        for prompt in [
            "",
            "raise limits",
            "lower limits to 4 machines",
            "raise limits to four machines",
            "raise limits to 0 machines",
            "raise limits to 101 machines",
            "raise limits to -4 machines",
            "raise limits to 4 machines please",
            "raise limits to 4 machines then run the build",
            "run the build and raise limits to 4 machines",
            "raise limits to 4 machines and 5 machines",
            "raise limits to 4 machines and",
            "raise limits to 10.555 EUR a day",
            "raise limits to 1,5 EUR a day",
            "raise limits to 10 EU a day",
            "raise limits to 10 EURO a day",
            "raise limits to 10 EUR",
            "raise limits to 10 EUR a week",
            "raise limits to 15 EUR a day and 20 EUR a day",
            "raise limits to 4 machines, 15 EUR a day",
        ] {
            assert_eq!(LimitGrant::parse(prompt), None, "{prompt:?}");
        }
    }

    #[test]
    fn the_grant_covers_exactly_what_it_names() {
        let grant = machines(4);
        let raise = |machines, spend| RequestedRaise {
            machines_at_once: machines,
            spend_per_day_micros: spend,
        };
        assert!(
            grant.covers(&raise(Some(4), None), "EUR"),
            "the named value"
        );
        assert!(
            grant.covers(&raise(Some(3), None), "EUR"),
            "below the named value"
        );
        assert!(
            !grant.covers(&raise(Some(5), None), "EUR"),
            "beyond the named value"
        );
        assert!(
            !grant.covers(&raise(None, Some(1)), "EUR"),
            "a field the prompt does not name"
        );
        assert!(
            !grant.covers(&raise(Some(4), Some(1)), "EUR"),
            "one uncovered field refuses the whole raise"
        );
        assert!(
            grant.covers(&raise(None, None), "EUR"),
            "nothing raised needs no cover"
        );
        let spend = LimitGrant::parse("raise limits to 15 EUR a day").unwrap();
        assert!(spend.covers(&raise(None, Some(15_000_000)), "EUR"));
        assert!(spend.covers(&raise(None, Some(15_000_000)), "eur"));
        assert!(!spend.covers(&raise(None, Some(15_000_001)), "EUR"));
        assert!(
            !spend.covers(&raise(None, Some(1)), "USD"),
            "another currency is not the named one"
        );
        assert!(
            !spend.covers(&raise(Some(2), Some(1)), "EUR"),
            "machines are not named"
        );
    }
}

#[cfg(test)]
mod scoped_authority_tests {
    use super::*;

    #[test]
    fn numeric_bounds_do_not_cross_the_named_scope_or_currency() {
        let grant = LimitGrant::parse("raise no-ask limits to 4 machines and 15 EUR a day").unwrap();
        let wanted = RequestedRaise { machines_at_once: Some(3), spend_per_day_micros: Some(14_000_000) };
        assert!(grant.covers_change(LimitScope::NoAsk, &wanted, "EUR", None));
        assert!(!grant.covers_change(LimitScope::Account, &wanted, "EUR", None));
        assert!(!grant.covers_change(LimitScope::NoAsk, &wanted, "USD", None));
        assert!(!grant.covers_change(LimitScope::NoAsk, &RequestedRaise { machines_at_once: Some(5), ..wanted }, "EUR", None));
        assert_eq!(LimitGrant::parse("raise limits to 4 machines").unwrap().scope, LimitScope::Account);
    }

    #[test]
    fn classes_are_exact_named_targets_without_case_folding_or_a_wildcard() {
        let classes = vec!["small".to_owned(), "GPU-A".to_owned()];
        let grant = LimitGrant::parse(r#"raise no-ask limits to classes ["small","GPU-A"]"#).unwrap();
        assert!(grant.covers_change(LimitScope::NoAsk, &RequestedRaise::default(), "EUR", Some(&classes)));
        assert!(!grant.covers_change(LimitScope::Account, &RequestedRaise::default(), "EUR", Some(&classes)));
        assert!(!grant.covers_change(LimitScope::NoAsk, &RequestedRaise::default(), "EUR", Some(&["small".into()])));
        assert!(!grant.covers_change(LimitScope::NoAsk, &RequestedRaise::default(), "EUR", Some(&["small".into(), "gpu-a".into()])));
        let numeric = LimitGrant::parse("raise no-ask limits to 4 machines").unwrap();
        assert!(!numeric.covers_change(LimitScope::NoAsk, &RequestedRaise::default(), "EUR", Some(&classes)));
        for prompt in [r#"raise account limits to classes ["small"]"#, r#"raise no-ask limits to classes ["small","small"]"#, r#"raise no-ask limits to classes ["small"] then run"#] {
            assert!(LimitGrant::parse(prompt).is_none(), "{prompt}");
        }
    }

    #[test]
    fn the_owner_step_round_trips_all_named_fields_and_does_not_round_spend() {
        let classes = vec!["small".to_owned(), "GPU-A".to_owned()];
        let wanted = RequestedRaise { machines_at_once: Some(4), spend_per_day_micros: Some(15_000_000) };
        let prompt = LimitGrant::prompt_for(LimitScope::NoAsk, &wanted, "EUR", Some(&classes)).unwrap();
        assert_eq!(prompt, r#"raise no-ask limits to 4 machines and 15.00 EUR a day and classes ["small","GPU-A"]"#);
        assert!(LimitGrant::parse(&prompt).unwrap().covers_change(LimitScope::NoAsk, &wanted, "EUR", Some(&classes)));
        assert!(LimitGrant::prompt_for(LimitScope::Account, &wanted, "EUR", Some(&classes)).is_none());
        assert!(LimitGrant::prompt_for(LimitScope::Account, &RequestedRaise { spend_per_day_micros: Some(15_000_001), ..wanted }, "EUR", None).is_none());
    }
}
