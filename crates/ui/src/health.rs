const FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Health {
    Healthy,
    Unhealthy(String),
}

/// Folds probe outcomes into health transitions: `record` reports a change
/// only, so callers can surface it without repeating themselves.
#[derive(Debug, Default)]
pub(crate) struct HealthTracker {
    failures: u32,
    last_reported: Option<Health>,
}

impl HealthTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record(&mut self, outcome: Result<(), String>) -> Option<Health> {
        match outcome {
            Ok(()) => {
                self.failures = 0;
                self.report(Health::Healthy)
            }
            Err(reason) => {
                self.failures = self.failures.saturating_add(1);
                if self.failures < FAILURE_THRESHOLD {
                    return None;
                }
                if matches!(self.last_reported, Some(Health::Unhealthy(_))) {
                    return None;
                }
                self.report(Health::Unhealthy(reason))
            }
        }
    }

    /// Returns nothing on purpose: the caller has already cleared health for
    /// the state that triggered the reset.
    pub(crate) fn reset(&mut self) {
        self.failures = 0;
        self.last_reported = None;
    }

    fn report(&mut self, health: Health) -> Option<Health> {
        if self.last_reported.as_ref() == Some(&health) {
            return None;
        }
        self.last_reported = Some(health.clone());
        Some(health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fail(tracker: &mut HealthTracker, reason: &str) -> Option<Health> {
        tracker.record(Err(reason.to_string()))
    }

    #[test]
    fn two_failures_report_nothing_third_reports_unhealthy() {
        let mut tracker = HealthTracker::new();
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(
            fail(&mut tracker, "refused"),
            Some(Health::Unhealthy("refused".to_string()))
        );
    }

    #[test]
    fn further_failures_stay_quiet() {
        let mut tracker = HealthTracker::new();
        for _ in 0..3 {
            fail(&mut tracker, "timeout");
        }
        for _ in 0..5 {
            assert_eq!(fail(&mut tracker, "other"), None);
        }
    }

    #[test]
    fn success_after_unhealthy_reports_healthy() {
        let mut tracker = HealthTracker::new();
        for _ in 0..3 {
            fail(&mut tracker, "timeout");
        }
        assert_eq!(tracker.record(Ok(())), Some(Health::Healthy));
        assert_eq!(tracker.record(Ok(())), None);
    }

    #[test]
    fn first_success_of_a_session_reports_healthy() {
        let mut tracker = HealthTracker::new();
        assert_eq!(tracker.record(Ok(())), Some(Health::Healthy));
        assert_eq!(tracker.record(Ok(())), None);
    }

    #[test]
    fn success_between_failures_resets_the_streak() {
        let mut tracker = HealthTracker::new();
        assert_eq!(tracker.record(Ok(())), Some(Health::Healthy));
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(tracker.record(Ok(())), None);
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(
            fail(&mut tracker, "timeout"),
            Some(Health::Unhealthy("timeout".to_string()))
        );
    }

    #[test]
    fn reset_clears_the_streak_and_reports_nothing() {
        let mut tracker = HealthTracker::new();
        for _ in 0..3 {
            fail(&mut tracker, "timeout");
        }
        tracker.reset();
        assert_eq!(fail(&mut tracker, "timeout"), None);
        assert_eq!(fail(&mut tracker, "timeout"), None);
        tracker.reset();
        assert_eq!(tracker.record(Ok(())), Some(Health::Healthy));
    }
}
