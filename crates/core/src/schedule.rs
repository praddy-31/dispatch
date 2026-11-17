//! When to refresh. News: every 3 hours by default. arXiv: once a day at a fixed UTC time (05:30 UTC, just
//! after arXiv rebuilds its feeds at midnight US Eastern: 04:00 UTC in summer time,
//! 05:00 UTC in winter). Instead of an alarm, the app asks "is the last good refresh older
//! than the latest slot?" every minute, so a Mac that slept through 05:30 UTC catches up on wake.

use chrono::{DateTime, Duration, NaiveTime, Utc};

pub const DEFAULT_SLOT_UTC: (u32, u32) = (5, 30);
/// News changes all day; arXiv once a day (and its feed is cached for 24 h).
pub const NEWS_EVERY: Duration = Duration::hours(3);
/// After a run where every source failed (offline), wait this long before trying again.
pub const RETRY_AFTER: Duration = Duration::minutes(10);

/// The most recent slot at or before `now`.
pub fn latest_slot(now: DateTime<Utc>, slot: (u32, u32)) -> DateTime<Utc> {
    let t = NaiveTime::from_hms_opt(slot.0, slot.1, 0).unwrap_or_default();
    let today = now.date_naive().and_time(t).and_utc();
    if today <= now {
        today
    } else {
        today - Duration::days(1)
    }
}

pub fn next_slot(now: DateTime<Utc>, slot: (u32, u32)) -> DateTime<Utc> {
    latest_slot(now, slot) + Duration::days(1)
}

/// True when an automatic daily refresh should run now.
pub fn due(
    last_ok: Option<DateTime<Utc>>,
    last_attempt: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    slot: (u32, u32),
) -> bool {
    due_days(last_ok, last_attempt, now, slot, 1)
}

/// Every `days` days at the slot: due when the last good refresh is older than the slot
/// `days - 1` days before the latest one.
pub fn due_days(
    last_ok: Option<DateTime<Utc>>,
    last_attempt: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    slot: (u32, u32),
    days: u32,
) -> bool {
    let slot_time = latest_slot(now, slot) - Duration::days(i64::from(days.max(1)) - 1);
    let stale = last_ok.is_none_or(|t| t < slot_time);
    let recently_tried = last_attempt.is_some_and(|t| t > slot_time && now - t < RETRY_AFTER);
    stale && !recently_tried
}

/// True when a source refreshed every `every` should run now. Same back-off as `due`.
pub fn due_every(
    last_ok: Option<DateTime<Utc>>,
    last_attempt: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    every: Duration,
) -> bool {
    let stale = last_ok.is_none_or(|t| now - t >= every);
    let recently_tried =
        last_attempt.is_some_and(|t| last_ok.is_none_or(|ok| t > ok) && now - t < RETRY_AFTER);
    stale && !recently_tried
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn latest_slot_is_today_after_the_slot_and_yesterday_before() {
        assert_eq!(
            latest_slot(at("2026-10-30T06:00:00Z"), (5, 30)),
            at("2026-10-30T05:30:00Z")
        );
        assert_eq!(
            latest_slot(at("2026-10-30T05:29:59Z"), (5, 30)),
            at("2026-10-29T05:30:00Z")
        );
        assert_eq!(
            next_slot(at("2026-10-30T06:00:00Z"), (5, 30)),
            at("2026-10-31T05:30:00Z")
        );
    }

    #[test]
    fn due_catches_up_after_sleep_and_backs_off_when_offline() {
        let slot = (5, 30);
        // Never refreshed: due.
        assert!(due(None, None, at("2026-10-30T01:00:00Z"), slot));
        // Refreshed after yesterday's slot, before today's: not due until 05:30.
        let last = Some(at("2026-10-29T06:00:00Z"));
        assert!(!due(last, None, at("2026-10-30T05:00:00Z"), slot));
        // Asleep through 05:30, woke at 09:00: due.
        assert!(due(last, None, at("2026-10-30T09:00:00Z"), slot));
        // Tried at 09:00 and everything failed: wait 10 minutes, then try again.
        let tried = Some(at("2026-10-30T09:00:00Z"));
        assert!(!due(last, tried, at("2026-10-30T09:05:00Z"), slot));
        assert!(due(last, tried, at("2026-10-30T09:10:00Z"), slot));
        // Refreshed today: not due again today.
        assert!(!due(
            Some(at("2026-10-30T09:10:00Z")),
            tried,
            at("2026-10-30T20:00:00Z"),
            slot
        ));
    }

    #[test]
    fn arxiv_every_two_days_skips_a_slot() {
        let slot = (5, 30);
        let last = Some(at("2026-10-29T06:00:00Z"));
        assert!(
            !due_days(last, None, at("2026-10-30T09:00:00Z"), slot, 2),
            "one day later"
        );
        assert!(
            due_days(last, None, at("2026-10-31T05:31:00Z"), slot, 2),
            "two slots later"
        );
        assert!(due_days(last, None, at("2026-10-30T09:00:00Z"), slot, 1));
    }

    #[test]
    fn news_every_three_hours_with_back_off() {
        let every = NEWS_EVERY;
        assert!(due_every(None, None, at("2026-10-30T01:00:00Z"), every));
        let ok = Some(at("2026-10-30T01:00:00Z"));
        assert!(!due_every(ok, None, at("2026-10-30T03:59:00Z"), every));
        assert!(due_every(ok, None, at("2026-10-30T04:00:00Z"), every));
        // Offline at 04:00: wait 10 minutes before the next try.
        let tried = Some(at("2026-10-30T04:00:00Z"));
        assert!(!due_every(ok, tried, at("2026-10-30T04:05:00Z"), every));
        assert!(due_every(ok, tried, at("2026-10-30T04:10:00Z"), every));
    }
}
