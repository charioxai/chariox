//! Every move of an agent's usage-limit stop, in one place.
//!
//! The stop is written by the failed turn, the person's decision, the one-second scheduler and a
//! send that went wrong, and they run at the same time. So no caller reads the stop and writes it
//! back: each move is a `StopEvent` that `transition` accepts or refuses against the stop held at
//! that moment, and the agent store applies it under its own lock (`change_usage_limit_stop`). A
//! refused event leaves the stop as it is. The table is the whole contract; each row has a test.
//!
//! | event          | held stop                                            | next stop                                    |
//! |----------------|------------------------------------------------------|----------------------------------------------|
//! | `Hit`          | none, Asking, Armed, Declined                        | a new stop: Armed when automatic, else Asking|
//! | `Hit`          | Resuming (that resume was refused)                   | Armed at the next retry time, else Asking    |
//! | `Arm`          | exactly the stop shown, Asking or Declined           | Armed at the time given                      |
//! | `Decline`      | exactly the stop shown, Asking or Armed              | Declined                                     |
//! | `ClaimDue`     | exactly the stop the scheduler selected, Armed, due  | Resuming, not sent                           |
//! | `ClaimDecided` | Asking, Armed or Declined                            | Resuming, not sent                           |
//! | `Sent`         | Resuming claimed at `since_ms`                       | Resuming, sent                               |
//! | `Retain`       | Resuming claimed at `since_ms`, not sent             | the stop the claim started from              |
//! | `Lost`         | exactly the stop the scheduler selected, Resuming,   | Armed now                                    |
//! |                | not sent (a kernel that went away took the claim)    |                                              |
//! | `Settle`       | exactly the stop the scheduler selected, Resuming,   | none (the resumed turn ended without the     |
//! |                | sent, and past `RESUME_SETTLE_MS`                    | limit: the agent is back)                    |
//! | `Drop`         | any stop of the stopped turn that was cancelled      | none                                         |
//! | `Continued`    | Asking, Armed or Declined, and the agent's turn runs | none (the agent went on, on another account: |
//! |                | on an account other than the stop's                  | the stop is over, whatever route it took)    |
//!
//! Every other pairing is refused. No row moves a Resuming stop to Declined, and the scheduler
//! never moves a Declined stop to Resuming, so a decision never overwrites a claim and a claim by
//! the scheduler never overwrites a decision. A person who chose to go on (`ClaimDecided`) may
//! resume a Declined stop: that is their later decision.

use crate::agent::{UsageLimitStop, UsageLimitStopState};

/// The provider named a reset time and still refused at it: wait this long after each refusal.
const RETRY_AFTER_NAMED_RESET_MS: [u64; 3] = [60_000, 300_000, 900_000];
/// The provider named no time: wait this long after the stop and after each refusal.
const RETRY_AFTER_UNNAMED_RESET_MS: [u64; 4] = [900_000, 1_800_000, 3_600_000, 7_200_000];
/// A resume that has not stopped again after this long worked.
const RESUME_SETTLE_MS: u64 = 30_000;

/// When the kernel resumes next: at the reset time the provider named, otherwise after a wait
/// that grows with each refusal; `None` once it has tried as often as it will.
pub(super) fn next_resume_at(
    now_ms: u64,
    named_reset_ms: Option<u64>,
    refused: u32,
) -> Option<u64> {
    match (named_reset_ms, refused) {
        (Some(reset), 0) => Some(reset.max(now_ms)),
        (Some(_), refused) => RETRY_AFTER_NAMED_RESET_MS
            .get(refused as usize - 1)
            .map(|wait| now_ms + wait),
        (None, refused) => RETRY_AFTER_UNNAMED_RESET_MS
            .get(refused as usize)
            .map(|wait| now_ms + wait),
    }
}

/// The turn that ended on the limit, as the stop records it.
pub(super) struct StopOrigin {
    pub prompt_id: String,
    pub provider: String,
    pub account_profile: String,
    pub account_label: String,
}

/// How the provider's usage limit reached a turn.
#[derive(Clone, Copy)]
pub(super) struct LimitHit {
    /// The time the provider's message named, when it named one.
    pub reset_at_ms: Option<u64>,
    /// The person lets the kernel resume by itself.
    pub automatic: bool,
    pub now: u64,
}

pub(super) enum StopEvent<'a> {
    /// A turn ended on the provider's usage limit.
    Hit {
        origin: &'a StopOrigin,
        hit: LimitHit,
    },
    /// The person, shown `shown`, lets the kernel resume at `at_ms`.
    Arm {
        shown: &'a UsageLimitStop,
        at_ms: u64,
    },
    /// The account's usage came back before the armed time: the resume is due now.
    Reset {
        selected: &'a UsageLimitStop,
        now: u64,
    },
    /// The person, or an agent for an armed resume, shown `shown`, leaves it stopped.
    Decline { shown: &'a UsageLimitStop },
    /// The scheduler claims the armed stop it selected.
    ClaimDue {
        selected: &'a UsageLimitStop,
        now: u64,
    },
    /// The person chose to go on, whatever the stop says now.
    ClaimDecided { now: u64 },
    /// The continuation claimed at `since_ms` was admitted.
    Sent { since_ms: u64 },
    /// The continuation claimed at `since_ms` could not be sent: the stop it started from is back.
    Retain {
        since_ms: u64,
        restore: &'a UsageLimitStop,
    },
    /// The claim was taken and never sent, and no turn was admitted: due again.
    Lost {
        selected: &'a UsageLimitStop,
        now: u64,
    },
    /// The resumed turn ended without the limit: the agent is back.
    Settle {
        selected: &'a UsageLimitStop,
        now: u64,
    },
    /// The task of the stopped turn was cancelled on purpose.
    Drop { prompt_id: &'a str },
    /// The agent's turn runs on `account_profile`, which is not the account that hit the limit:
    /// it went on by a settings change, a prompt or a decision. `account_label` names it for the
    /// history.
    Continued {
        account_profile: &'a str,
        account_label: &'a str,
    },
}

/// How a turn that ended on the usage limit goes on, given the stop held when it ended.
pub(super) struct AfterLimit {
    pub named_reset: Option<u64>,
    pub refusals: u32,
    pub stopped_at_ms: u64,
    pub state: UsageLimitStopState,
}

impl AfterLimit {
    pub(super) fn of(held: Option<&UsageLimitStop>, hit: LimitHit) -> Self {
        let LimitHit {
            reset_at_ms,
            automatic,
            now,
        } = hit;
        // A stop while a resume runs is that resume refused.
        let resumed_before =
            held.filter(|stop| matches!(stop.state, UsageLimitStopState::Resuming { .. }));
        let refusals = resumed_before.map_or(0, |stop| stop.refused + 1);
        let named_reset = match resumed_before {
            Some(stop) => stop.resumes_at_ms,
            None => reset_at_ms.filter(|at| *at > now),
        };
        let state = if automatic || resumed_before.is_some() {
            match next_resume_at(now, named_reset, refusals) {
                Some(at_ms) => UsageLimitStopState::Armed { at_ms },
                None => UsageLimitStopState::Asking,
            }
        } else {
            UsageLimitStopState::Asking
        };
        Self {
            named_reset,
            refusals,
            stopped_at_ms: resumed_before.map_or(now, |stop| stop.stopped_at_ms),
            state,
        }
    }
}

/// The stop after `event`, `Some(None)` when it ends, or `None` when the event does not apply to
/// the stop held now.
pub(super) fn transition(
    held: Option<&UsageLimitStop>,
    event: &StopEvent<'_>,
) -> Option<Option<UsageLimitStop>> {
    use UsageLimitStopState::{Armed, Asking, Declined, Resuming};
    if let StopEvent::Hit { origin, hit } = event {
        let after = AfterLimit::of(held, *hit);
        return Some(Some(UsageLimitStop {
            prompt_id: origin.prompt_id.clone(),
            provider: origin.provider.clone(),
            account_profile: origin.account_profile.clone(),
            account_label: origin.account_label.clone(),
            stopped_at_ms: after.stopped_at_ms,
            resumes_at_ms: after.named_reset,
            refused: after.refusals,
            state: after.state,
        }));
    }
    let held = held?;
    let with = |state| {
        Some(Some(UsageLimitStop {
            state,
            ..held.clone()
        }))
    };
    match (event, &held.state) {
        (StopEvent::Arm { shown, at_ms }, Asking | Declined) if held == *shown => {
            with(Armed { at_ms: *at_ms })
        }
        (StopEvent::Reset { selected, now }, Armed { at_ms })
            if held == *selected && at_ms > now =>
        {
            with(Armed { at_ms: *now })
        }
        (StopEvent::Decline { shown }, Asking | Armed { .. }) if held == *shown => with(Declined),
        (StopEvent::ClaimDue { selected, now }, Armed { at_ms })
            if held == *selected && at_ms <= now =>
        {
            with(Resuming {
                since_ms: *now,
                sent: false,
            })
        }
        (StopEvent::ClaimDecided { now }, Asking | Armed { .. } | Declined) => with(Resuming {
            since_ms: *now,
            sent: false,
        }),
        (
            StopEvent::Sent { since_ms },
            Resuming {
                since_ms: claimed, ..
            },
        ) if claimed == since_ms => with(Resuming {
            since_ms: *since_ms,
            sent: true,
        }),
        (
            StopEvent::Retain { since_ms, restore },
            Resuming {
                since_ms: claimed,
                sent: false,
            },
        ) if claimed == since_ms => Some(Some((*restore).clone())),
        (StopEvent::Lost { selected, now }, Resuming { sent: false, .. }) if held == *selected => {
            with(Armed { at_ms: *now })
        }
        (
            StopEvent::Settle { selected, now },
            Resuming {
                since_ms,
                sent: true,
            },
        ) if held == *selected && now.saturating_sub(*since_ms) > RESUME_SETTLE_MS => Some(None),
        (StopEvent::Drop { prompt_id }, _) if held.prompt_id == *prompt_id => Some(None),
        (
            StopEvent::Continued {
                account_profile, ..
            },
            Asking | Armed { .. } | Declined,
        ) if held.account_profile != *account_profile => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use UsageLimitStopState::{Armed, Asking, Declined, Resuming};

    fn stop(state: UsageLimitStopState) -> UsageLimitStop {
        UsageLimitStop {
            prompt_id: "p1".into(),
            provider: "claude".into(),
            account_profile: "default".into(),
            account_label: "main".into(),
            stopped_at_ms: 1,
            resumes_at_ms: Some(5_000),
            refused: 0,
            state,
        }
    }

    fn origin() -> StopOrigin {
        StopOrigin {
            prompt_id: "p2".into(),
            provider: "claude".into(),
            account_profile: "default".into(),
            account_label: "main".into(),
        }
    }

    fn states() -> Vec<Option<UsageLimitStop>> {
        vec![
            None,
            Some(stop(Asking)),
            Some(stop(Armed { at_ms: 5_000 })),
            Some(stop(Resuming {
                since_ms: 5_000,
                sent: false,
            })),
            Some(stop(Resuming {
                since_ms: 5_000,
                sent: true,
            })),
            Some(stop(Declined)),
        ]
    }

    /// Which of the six held stops (none, Asking, Armed, Resuming unsent, Resuming sent, Declined)
    /// the event applies to.
    fn applies_to(event: &StopEvent<'_>) -> [bool; 6] {
        let mut row = [false; 6];
        for (i, held) in states().iter().enumerate() {
            row[i] = transition(held.as_ref(), event).is_some();
        }
        row
    }

    fn next(held: &UsageLimitStop, event: &StopEvent<'_>) -> Option<UsageLimitStop> {
        transition(Some(held), event).expect("the event applies")
    }

    #[test]
    fn a_limit_hit_arms_a_stop_when_automatic_and_asks_otherwise_whatever_was_held_but_a_resume() {
        let o = origin();
        let automatic = StopEvent::Hit {
            origin: &o,
            hit: LimitHit {
                reset_at_ms: Some(9_000),
                automatic: true,
                now: 1_000,
            },
        };
        let asking = StopEvent::Hit {
            origin: &o,
            hit: LimitHit {
                reset_at_ms: Some(9_000),
                automatic: false,
                now: 1_000,
            },
        };
        for held in [
            None,
            Some(stop(Asking)),
            Some(stop(Armed { at_ms: 5_000 })),
            Some(stop(Declined)),
        ] {
            let armed = transition(held.as_ref(), &automatic).unwrap().unwrap();
            assert_eq!(armed.state, Armed { at_ms: 9_000 });
            assert_eq!(
                (armed.refused, armed.stopped_at_ms, armed.prompt_id.as_str()),
                (0, 1_000, "p2")
            );
            let asked = transition(held.as_ref(), &asking).unwrap().unwrap();
            assert_eq!(asked.state, Asking);
        }
        assert_eq!(
            applies_to(&automatic),
            [true; 6],
            "a failed turn is always recorded"
        );
    }

    #[test]
    fn a_limit_hit_while_a_resume_runs_is_that_resume_refused_and_retried_a_few_times() {
        let o = origin();
        let hit = |now| StopEvent::Hit {
            origin: &o,
            hit: LimitHit {
                reset_at_ms: None,
                automatic: false,
                now,
            },
        };
        let mut held = stop(Resuming {
            since_ms: 5_000,
            sent: true,
        });
        let retried = next(&held, &hit(10_000)).unwrap();
        assert_eq!(retried.state, Armed { at_ms: 70_000 });
        assert_eq!(
            (
                retried.refused,
                retried.stopped_at_ms,
                retried.resumes_at_ms
            ),
            (1, 1, Some(5_000))
        );
        held.refused = 3;
        assert_eq!(
            next(&held, &hit(10_000)).unwrap().state,
            Asking,
            "it asks after the last retry"
        );
    }

    /// Which of the six held stops an event shown that very stop applies to.
    fn applies_when_shown(event: fn(&UsageLimitStop) -> StopEvent<'_>) -> [bool; 6] {
        let mut row = [false; 6];
        for (i, held) in states().iter().enumerate() {
            row[i] = held
                .as_ref()
                .is_some_and(|held| transition(Some(held), &event(held)).is_some());
        }
        row
    }

    #[test]
    fn arming_applies_to_the_asking_or_declined_stop_shown_and_to_nothing_that_moved_on() {
        fn arm(shown: &UsageLimitStop) -> StopEvent<'_> {
            StopEvent::Arm {
                shown,
                at_ms: 7_000,
            }
        }
        assert_eq!(
            applies_when_shown(arm),
            [false, true, false, false, false, true]
        );
        assert_eq!(
            next(&stop(Declined), &arm(&stop(Declined))).unwrap().state,
            Armed { at_ms: 7_000 }
        );
        // Shown Asking, and a decline or a claim landed first.
        let shown = stop(Asking);
        assert!(transition(Some(&stop(Declined)), &arm(&shown)).is_none());
        assert!(transition(
            Some(&stop(Resuming {
                since_ms: 1,
                sent: false
            })),
            &arm(&shown)
        )
        .is_none());
    }

    #[test]
    fn declining_applies_to_the_asking_or_armed_stop_shown_so_it_never_overwrites_a_claim() {
        fn decline(shown: &UsageLimitStop) -> StopEvent<'_> {
            StopEvent::Decline { shown }
        }
        assert_eq!(
            applies_when_shown(decline),
            [false, true, true, false, false, false]
        );
        let armed = stop(Armed { at_ms: 5_000 });
        assert_eq!(next(&armed, &decline(&armed)).unwrap().state, Declined);
        // The scheduler claimed the stop the person was shown: the decision is refused.
        let claimed = stop(Resuming {
            since_ms: 5_000,
            sent: false,
        });
        assert!(transition(Some(&claimed), &decline(&armed)).is_none());
        // The stop was armed for another time since: also not the stop shown.
        assert!(transition(Some(&stop(Armed { at_ms: 9_000 })), &decline(&armed)).is_none());
    }

    #[test]
    fn the_scheduler_claims_only_the_exact_stop_it_selected_while_it_is_armed_and_due() {
        let selected = stop(Armed { at_ms: 5_000 });
        let claim = |now| StopEvent::ClaimDue {
            selected: &selected,
            now,
        };
        assert_eq!(
            applies_to(&claim(5_000)),
            [false, false, true, false, false, false]
        );
        assert_eq!(
            next(&selected, &claim(5_000)).unwrap().state,
            Resuming {
                since_ms: 5_000,
                sent: false
            }
        );
        assert!(
            transition(Some(&selected), &claim(4_999)).is_none(),
            "not due yet"
        );
        // Armed again for later, or armed by another stopped turn, is not the stop it selected.
        let later = stop(Armed { at_ms: 9_000 });
        assert!(transition(Some(&later), &claim(9_000)).is_none());
        let other = UsageLimitStop {
            prompt_id: "p9".into(),
            ..selected.clone()
        };
        assert!(transition(Some(&other), &claim(5_000)).is_none());
    }

    #[test]
    fn a_decided_claim_applies_to_every_stop_a_resume_has_not_taken() {
        let claim = StopEvent::ClaimDecided { now: 6_000 };
        assert_eq!(applies_to(&claim), [false, true, true, false, false, true]);
        assert_eq!(
            next(&stop(Declined), &claim).unwrap().state,
            Resuming {
                since_ms: 6_000,
                sent: false
            }
        );
    }

    #[test]
    fn a_sent_continuation_marks_only_the_claim_it_was_sent_for() {
        let sent = StopEvent::Sent { since_ms: 5_000 };
        assert_eq!(applies_to(&sent), [false, false, false, true, true, false]);
        assert_eq!(
            next(
                &stop(Resuming {
                    since_ms: 5_000,
                    sent: false
                }),
                &sent
            )
            .unwrap()
            .state,
            Resuming {
                since_ms: 5_000,
                sent: true
            }
        );
        let other_claim = stop(Resuming {
            since_ms: 6_000,
            sent: false,
        });
        assert!(transition(Some(&other_claim), &sent).is_none());
    }

    #[test]
    fn a_continuation_that_could_not_be_sent_restores_the_stop_its_claim_started_from() {
        let armed = stop(Armed { at_ms: 5_000 });
        let retain = StopEvent::Retain {
            since_ms: 5_000,
            restore: &armed,
        };
        assert_eq!(
            applies_to(&retain),
            [false, false, false, true, false, false]
        );
        assert_eq!(
            transition(
                Some(&stop(Resuming {
                    since_ms: 5_000,
                    sent: false
                })),
                &retain
            ),
            Some(Some(armed.clone()))
        );
        // Another claim's resume, or one a person's decision or a new limit moved on, is not its own.
        let other_claim = stop(Resuming {
            since_ms: 6_000,
            sent: false,
        });
        assert!(transition(Some(&other_claim), &retain).is_none());
        assert!(transition(Some(&stop(Declined)), &retain).is_none());
    }

    #[test]
    fn a_claim_a_kernel_took_and_never_sent_is_due_again_only_for_the_stop_the_scheduler_selected()
    {
        let selected = stop(Resuming {
            since_ms: 5_000,
            sent: false,
        });
        let lost = StopEvent::Lost {
            selected: &selected,
            now: 8_000,
        };
        assert_eq!(applies_to(&lost), [false, false, false, true, false, false]);
        assert_eq!(
            next(&selected, &lost).unwrap().state,
            Armed { at_ms: 8_000 }
        );
        let sent_meanwhile = stop(Resuming {
            since_ms: 5_000,
            sent: true,
        });
        assert!(
            transition(Some(&sent_meanwhile), &lost).is_none(),
            "a send recorded since is not lost"
        );
    }

    #[test]
    fn a_resume_that_settled_ends_the_stop_only_for_the_stop_the_scheduler_selected() {
        let selected = stop(Resuming {
            since_ms: 5_000,
            sent: true,
        });
        let settle = |now| StopEvent::Settle {
            selected: &selected,
            now,
        };
        assert_eq!(
            applies_to(&settle(40_000)),
            [false, false, false, false, true, false]
        );
        assert_eq!(transition(Some(&selected), &settle(40_000)), Some(None));
        assert!(
            transition(Some(&selected), &settle(35_000)).is_none(),
            "not settled yet"
        );
        let armed_again = stop(Armed { at_ms: 9_000 });
        assert!(
            transition(Some(&armed_again), &settle(40_000)).is_none(),
            "it stopped again meanwhile"
        );
    }

    #[test]
    fn a_task_cancelled_on_purpose_ends_every_stop_of_its_turn_and_no_other() {
        let drop = StopEvent::Drop { prompt_id: "p1" };
        assert_eq!(applies_to(&drop), [false, true, true, true, true, true]);
        assert_eq!(
            transition(
                Some(&stop(Resuming {
                    since_ms: 1,
                    sent: false
                })),
                &drop
            ),
            Some(None)
        );
        let other_turn = StopEvent::Drop { prompt_id: "p9" };
        assert_eq!(
            applies_to(&other_turn),
            [false; 6],
            "a newer stopped turn is not its stop"
        );
    }

    #[test]
    fn a_turn_on_another_account_ends_a_stop_nobody_took() {
        let elsewhere = StopEvent::Continued {
            account_profile: "other",
            account_label: "Other",
        };
        assert_eq!(
            applies_to(&elsewhere),
            [false, true, true, false, false, true]
        );
        let same = StopEvent::Continued {
            account_profile: "default",
            account_label: "main",
        };
        assert_eq!(
            applies_to(&same),
            [false; 6],
            "a turn on the limited account does not end its stop"
        );
    }

    #[test]
    fn a_named_reset_is_waited_for_and_then_retried_a_few_times() {
        assert_eq!(next_resume_at(1_000, Some(5_000), 0), Some(5_000));
        assert_eq!(next_resume_at(9_000, Some(5_000), 0), Some(9_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 1), Some(70_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 2), Some(310_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 3), Some(910_000));
        assert_eq!(next_resume_at(10_000, Some(5_000), 4), None);
    }

    #[test]
    fn an_unnamed_reset_backs_off_and_then_asks() {
        assert_eq!(next_resume_at(0, None, 0), Some(900_000));
        assert_eq!(next_resume_at(0, None, 1), Some(1_800_000));
        assert_eq!(next_resume_at(0, None, 3), Some(7_200_000));
        assert_eq!(next_resume_at(0, None, 4), None);
    }
}
