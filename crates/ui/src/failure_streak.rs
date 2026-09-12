/// Tracks one failure streak: `record_failure` reports true exactly once per
/// streak, so callers can toast on the clean -> failing transition and stay
/// quiet while a failure repeats.
#[derive(Default)]
pub(crate) struct FailureStreak {
    failed: bool,
}

impl FailureStreak {
    pub(crate) fn new() -> Self {
        Self { failed: false }
    }

    pub(crate) fn record_failure(&mut self) -> bool {
        let first = !self.failed;
        self.failed = true;
        first
    }

    pub(crate) fn record_success(&mut self) {
        self.failed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_toasts_once_per_streak() {
        let mut streak = FailureStreak::new();
        assert!(streak.record_failure());
        assert!(!streak.record_failure());
        assert!(!streak.record_failure());
        streak.record_success();
        assert!(streak.record_failure());
    }

    #[test]
    fn success_while_clean_stays_clean() {
        let mut streak = FailureStreak::new();
        streak.record_success();
        assert!(streak.record_failure());
    }
}
