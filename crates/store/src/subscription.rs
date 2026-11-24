//! Policy shared by the GTK and native macOS subscription schedulers.

use std::collections::{HashMap, HashSet};

use nolurix_config::{profile::Group, Settings};

const RETRY_SECONDS: i64 = 5 * 60;

#[derive(Default)]
pub struct RefreshSchedule {
    last_attempt: HashMap<i64, i64>,
    startup_pending: HashSet<i64>,
}

fn eligible(group: &Group) -> bool {
    group.is_subscription() && !group.archive && !group.skip_auto_update
}

impl RefreshSchedule {
    pub fn on_startup(groups: &[Group], settings: &Settings) -> Self {
        let mut schedule = Self::default();
        if settings.sub_auto_update {
            schedule.startup_pending = groups
                .iter()
                .filter(|group| eligible(group))
                .map(|group| group.id)
                .collect();
        }
        schedule
    }

    pub fn due(&mut self, groups: &[Group], settings: &Settings, now: i64) -> Option<i64> {
        if !settings.sub_auto_update {
            return None;
        }
        let interval = settings.sub_auto_update_minutes.clamp(10, 10080) * 60;
        groups
            .iter()
            .filter(|group| {
                eligible(group)
                    && (self.startup_pending.contains(&group.id)
                        || group.sub_last_update <= 0
                        || group.sub_last_update > now
                        || now.saturating_sub(group.sub_last_update) >= interval)
                    && self.last_attempt.get(&group.id).is_none_or(|&attempt| {
                        attempt > now || now.saturating_sub(attempt) >= RETRY_SECONDS
                    })
            })
            .min_by_key(|group| {
                (
                    !self.startup_pending.contains(&group.id),
                    group.sub_last_update,
                )
            })
            .map(|group| group.id)
    }

    pub fn record_attempt(&mut self, gid: i64, now: i64) {
        self.startup_pending.remove(&gid);
        self.last_attempt
            .retain(|_, attempt| *attempt <= now && now.saturating_sub(*attempt) < RETRY_SECONDS);
        self.last_attempt.insert(gid, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: i64, sub_last_update: i64) -> Group {
        Group {
            id,
            url: "https://example.test/sub".to_string(),
            sub_last_update,
            ..Group::default()
        }
    }

    #[test]
    fn picks_oldest_eligible_group_and_respects_success_interval() {
        let settings = Settings {
            sub_auto_update: true,
            sub_auto_update_minutes: 10,
            ..Settings::default()
        };
        let mut groups = [
            Group::default(),
            group(1, 0),
            group(2, 0),
            group(3, 700),
            group(4, 500),
            group(5, 400),
        ];
        groups[1].archive = true;
        groups[2].skip_auto_update = true;
        let mut schedule = RefreshSchedule::default();
        assert_eq!(schedule.due(&groups, &Settings::default(), 1100), None);
        assert_eq!(schedule.due(&groups, &settings, 1100), Some(5));
        groups[5].sub_last_update = 1100;
        assert_eq!(schedule.due(&groups, &settings, 1100), Some(4));
        groups[4].sub_last_update = 1100;
        assert_eq!(schedule.due(&groups, &settings, 1100), None);
        assert_eq!(schedule.due(&groups, &settings, 1300), Some(3));
    }

    #[test]
    fn backs_off_failed_attempts_without_blocking_other_groups() {
        let settings = Settings {
            sub_auto_update: true,
            ..Settings::default()
        };
        let groups = [group(1, 0), group(2, 0)];
        let mut schedule = RefreshSchedule::default();
        assert_eq!(schedule.due(&groups, &settings, 1000), Some(1));
        schedule.record_attempt(1, 1000);
        assert_eq!(schedule.due(&groups, &settings, 1001), Some(2));
        schedule.record_attempt(2, 1001);
        assert_eq!(schedule.due(&groups, &settings, 1299), None);
        assert_eq!(schedule.due(&groups, &settings, 1300), Some(1));
    }

    #[test]
    fn refreshes_recent_subscriptions_once_per_launch() {
        let settings = Settings {
            sub_auto_update: true,
            sub_auto_update_minutes: 10,
            ..Settings::default()
        };
        let mut groups = [group(1, 900), group(2, 950), group(3, 0), group(4, 0)];
        groups[2].archive = true;
        groups[3].skip_auto_update = true;
        let mut schedule = RefreshSchedule::on_startup(&groups, &settings);
        assert_eq!(schedule.due(&groups, &Settings::default(), 1000), None);
        assert_eq!(schedule.due(&groups, &settings, 1000), Some(1));
        schedule.record_attempt(1, 1000);
        groups[0].sub_last_update = 1000;
        assert_eq!(schedule.due(&groups, &settings, 1001), Some(2));
        schedule.record_attempt(2, 1001);
        groups[1].sub_last_update = 1001;
        assert_eq!(schedule.due(&groups, &settings, 1301), None);
        assert_eq!(schedule.due(&groups, &settings, 1600), Some(1));
        assert_eq!(
            RefreshSchedule::on_startup(&groups, &settings).due(&groups, &settings, 1301),
            Some(1)
        );
        assert_eq!(
            RefreshSchedule::on_startup(&groups, &Settings::default())
                .due(&groups, &settings, 1002),
            None
        );
    }
}
