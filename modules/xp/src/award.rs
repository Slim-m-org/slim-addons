//! What a voice session or a message is worth, as pure functions.

use crate::settings::Settings;

const MINUTE_MS: i64 = 60_000;
/// One session never pays more than this, so a bad row cannot overflow a total.
const MAX_SESSION_XP: i64 = 1 << 40;

/// A closed voice session as `log.read` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub user_id: String,
    pub joined_at_ms: i64,
    pub left_at_ms: i64,
    pub alone_ms: i64,
}

impl Session {
    pub fn duration_ms(&self) -> i64 {
        self.left_at_ms.saturating_sub(self.joined_at_ms).max(0)
    }
}

/// XP for whole minutes of the session, counting only time with others when the setting says so.
pub fn session_xp(session: &Session, settings: &Settings) -> i64 {
    let duration = session.duration_ms();
    let alone = session.alone_ms.clamp(0, duration);
    let counted = if settings.only_with_others { duration - alone } else { duration };
    (counted / MINUTE_MS).saturating_mul(settings.xp_per_minute).min(MAX_SESSION_XP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(joined: i64, left: i64, alone: i64) -> Session {
        Session { id: 1, user_id: "u".into(), joined_at_ms: joined, left_at_ms: left, alone_ms: alone }
    }

    fn settings(per_minute: i64, only_with_others: bool) -> Settings {
        Settings { xp_per_minute: per_minute, only_with_others, ..Settings::default() }
    }

    #[test]
    fn whole_minutes_pay_the_per_minute_rate() {
        assert_eq!(session_xp(&session(0, 10 * MINUTE_MS, 0), &settings(5, true)), 50);
    }

    #[test]
    fn a_partial_minute_pays_nothing_and_a_zero_length_session_pays_nothing() {
        assert_eq!(session_xp(&session(0, MINUTE_MS - 1, 0), &settings(5, true)), 0);
        assert_eq!(session_xp(&session(7, 7, 0), &settings(5, true)), 0);
        assert_eq!(session_xp(&session(9, 3, 0), &settings(5, true)), 0);
    }

    #[test]
    fn time_alone_earns_nothing_when_only_with_others() {
        let s = session(0, 10 * MINUTE_MS, 4 * MINUTE_MS);
        assert_eq!(session_xp(&s, &settings(5, true)), 30);
        assert_eq!(session_xp(&s, &settings(5, false)), 50);
        assert_eq!(session_xp(&session(0, 10 * MINUTE_MS, 10 * MINUTE_MS), &settings(5, true)), 0);
    }

    #[test]
    fn an_alone_time_longer_than_the_session_or_negative_is_clamped() {
        assert_eq!(session_xp(&session(0, 5 * MINUTE_MS, 99 * MINUTE_MS), &settings(5, true)), 0);
        assert_eq!(session_xp(&session(0, 5 * MINUTE_MS, -1), &settings(5, true)), 25);
    }

    #[test]
    fn a_huge_session_is_capped_instead_of_overflowing() {
        let huge = session(0, i64::MAX, 0);
        assert_eq!(session_xp(&huge, &settings(100, true)), MAX_SESSION_XP);
        assert_eq!(session_xp(&session(i64::MIN, i64::MAX, 0), &settings(100, true)), MAX_SESSION_XP);
    }

    #[test]
    fn a_rate_of_zero_pays_nothing() {
        assert_eq!(session_xp(&session(0, 99 * MINUTE_MS, 0), &settings(0, true)), 0);
    }
}
