//! Row 569: what a `/sudo` prompt grants about the account's machine limits.
//!
//! The owner types `/sudo raise limits to 4 machines`; the kernel reads those words once, when
//! the prompt is submitted, into this structured grant and keeps the structure on the sudo
//! window. Authority never re-reads text. The grammar is strict: the whole prompt is the grant,
//! so a look-alike or a longer sentence grants nothing and the agent raises through a card.
//!
//! The parsed grant lives on the window (`KernelSudoTurn.limit_grant`, kernel memory only), and
//! `machine_acquisition::limits` reads it when an agent raises a limit.

/// The raise the owner named. A field the prompt does not name is not granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitGrant {
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
        let lowered = prompt.trim().trim_end_matches('.').to_lowercase();
        let words: Vec<&str> = lowered.split_whitespace().collect();
        let rest = words.strip_prefix(&["raise", "limits", "to"])?;
        let mut grant = LimitGrant {
            machines_at_once: None,
            spend_per_day: None,
        };
        for clause in rest.split(|word| *word == "and") {
            match clause {
                [count, "machine" | "machines"] => {
                    if grant.machines_at_once.is_some() {
                        return None;
                    }
                    let count: u32 = count
                        .parse()
                        .ok()
                        .filter(|n| (1..=MAX_MACHINES).contains(n))?;
                    grant.machines_at_once = Some(count);
                }
                [amount, currency, "a" | "per", "day"] => {
                    if grant.spend_per_day.is_some() {
                        return None;
                    }
                    let micros = spend_micros(amount)?;
                    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
                        return None;
                    }
                    grant.spend_per_day = Some((micros, currency.to_uppercase()));
                }
                _ => return None,
            }
        }
        Some(grant)
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
                machines_at_once: None,
                spend_per_day: spend(15_000_000, "EUR")
            })
        );
        assert_eq!(
            LimitGrant::parse("raise limits to 10.5 USD per day"),
            Some(LimitGrant {
                machines_at_once: None,
                spend_per_day: spend(10_500_000, "USD")
            })
        );
        let both = LimitGrant {
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
