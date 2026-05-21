use anyhow::{Context, Result, bail};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use roder_api::automations::{AutomationDefinition, AutomationSchedule, CatchUpPolicy};
use std::str::FromStr;
use time::OffsetDateTime;

use crate::model::{OccurrenceAction, ScheduledOccurrence, occurrence_key};

const MAX_EXPANSION: usize = 10_000;

pub fn next_after(
    schedule: &AutomationSchedule,
    after: OffsetDateTime,
    inclusive: bool,
) -> Result<Option<OffsetDateTime>> {
    match schedule {
        AutomationSchedule::Cron {
            expression,
            timezone,
        } => {
            let tz: Tz = timezone
                .parse()
                .with_context(|| format!("invalid cron timezone `{timezone}`"))?;
            let cron = Cron::from_str(expression)
                .with_context(|| format!("invalid cron expression `{expression}`"))?;
            let start = utc_to_tz(after, tz)?;
            let next = cron.find_next_occurrence(&start, inclusive)?;
            Ok(Some(offset_from_unix(next.timestamp())?))
        }
        AutomationSchedule::Interval { seconds } => {
            if *seconds == 0 {
                bail!("interval schedule seconds must be greater than zero");
            }
            let after_ts = after.unix_timestamp();
            let step = *seconds as i64;
            let next = if inclusive && after_ts % step == 0 {
                after_ts
            } else {
                after_ts - after_ts.rem_euclid(step) + step
            };
            Ok(Some(offset_from_unix(next)?))
        }
        AutomationSchedule::OneShot { run_at } => {
            let due = if inclusive {
                *run_at >= after
            } else {
                *run_at > after
            };
            Ok(due.then_some(*run_at))
        }
    }
}

pub fn expand_missed_occurrences(
    definition: &AutomationDefinition,
    last_checked_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<Vec<ScheduledOccurrence>> {
    if !definition.enabled || now <= last_checked_at {
        return Ok(Vec::new());
    }

    let mut due = Vec::new();
    let mut cursor = last_checked_at;
    for index in 0..MAX_EXPANSION {
        let Some(next) = next_after(&definition.schedule, cursor, false)? else {
            break;
        };
        if next > now {
            break;
        }
        due.push(next);
        cursor = next;
        if matches!(definition.schedule, AutomationSchedule::OneShot { .. })
            || index + 1 == MAX_EXPANSION
        {
            break;
        }
    }

    Ok(apply_catch_up(definition, due, now))
}

fn apply_catch_up(
    definition: &AutomationDefinition,
    mut due: Vec<OffsetDateTime>,
    now: OffsetDateTime,
) -> Vec<ScheduledOccurrence> {
    match definition.catch_up {
        CatchUpPolicy::RunAllMissed { max_per_tick } => {
            due.truncate(max_per_tick as usize);
            due.into_iter()
                .map(|scheduled_for| occurrence(definition, scheduled_for, OccurrenceAction::Run))
                .collect()
        }
        CatchUpPolicy::RunLatestOnly => {
            let Some(latest) = due.pop() else {
                return Vec::new();
            };
            let mut occurrences = due
                .into_iter()
                .map(|scheduled_for| {
                    occurrence(
                        definition,
                        scheduled_for,
                        OccurrenceAction::Skip {
                            reason: "coalesced_by_run_latest_only".to_string(),
                        },
                    )
                })
                .collect::<Vec<_>>();
            occurrences.push(occurrence(definition, latest, OccurrenceAction::Run));
            occurrences
        }
        CatchUpPolicy::SkipExpired { grace_seconds } => due
            .into_iter()
            .map(|scheduled_for| {
                let expired =
                    now.unix_timestamp() - scheduled_for.unix_timestamp() > grace_seconds as i64;
                if expired {
                    occurrence(
                        definition,
                        scheduled_for,
                        OccurrenceAction::Skip {
                            reason: "expired_by_catch_up_grace".to_string(),
                        },
                    )
                } else {
                    occurrence(definition, scheduled_for, OccurrenceAction::Run)
                }
            })
            .collect(),
    }
}

fn occurrence(
    definition: &AutomationDefinition,
    scheduled_for: OffsetDateTime,
    action: OccurrenceAction,
) -> ScheduledOccurrence {
    ScheduledOccurrence {
        automation_id: definition.id.clone(),
        occurrence_key: occurrence_key(&definition.id, scheduled_for),
        scheduled_for,
        action,
    }
}

fn utc_to_tz(value: OffsetDateTime, tz: Tz) -> Result<chrono::DateTime<Tz>> {
    let utc = Utc
        .timestamp_opt(value.unix_timestamp(), 0)
        .single()
        .context("invalid unix timestamp")?;
    Ok(utc.with_timezone(&tz))
}

fn offset_from_unix(timestamp: i64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(timestamp).context("invalid unix timestamp")
}
