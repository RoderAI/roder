use roder_api::transcript::UserMessage;
use time::OffsetDateTime;

const DEFAULT_FINALIZATION_RESERVE_SECONDS: u64 = 30;

pub(crate) fn finalization_reserve_seconds(turn_deadline_seconds: Option<u64>) -> u64 {
    let Some(seconds) = turn_deadline_seconds else {
        return DEFAULT_FINALIZATION_RESERVE_SECONDS;
    };
    if seconds >= DEFAULT_FINALIZATION_RESERVE_SECONDS * 4 {
        DEFAULT_FINALIZATION_RESERVE_SECONDS
    } else {
        (seconds / 4).clamp(1, DEFAULT_FINALIZATION_RESERVE_SECONDS)
    }
}

pub(crate) fn should_start_finalization(
    deadline: Option<OffsetDateTime>,
    reserve_seconds: u64,
    already_requested: bool,
) -> Option<u64> {
    if already_requested {
        return None;
    }
    let remaining = crate::runtime::deadline_remaining_seconds(deadline)?;
    (remaining <= reserve_seconds).then_some(remaining)
}

pub(crate) fn finalization_message(remaining_seconds: u64) -> UserMessage {
    UserMessage::text(format!(
        "Eval deadline finalization: {remaining_seconds} seconds remain before the turn deadline. Stop using tools and provide the final answer now from the current workspace state. If the task is incomplete, give the best final result available and do not start more commands."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    #[test]
    fn finalization_reserve_scales_down_for_short_deadlines() {
        assert_eq!(finalization_reserve_seconds(None), 30);
        assert_eq!(finalization_reserve_seconds(Some(870)), 30);
        assert_eq!(finalization_reserve_seconds(Some(60)), 15);
        assert_eq!(finalization_reserve_seconds(Some(3)), 1);
    }

    #[test]
    fn finalization_starts_once_inside_reserve() {
        let deadline = OffsetDateTime::now_utc() + Duration::seconds(2);
        assert!(should_start_finalization(Some(deadline), 5, false).is_some());
        assert_eq!(should_start_finalization(Some(deadline), 5, true), None);
        assert_eq!(should_start_finalization(None, 5, false), None);
    }
}
