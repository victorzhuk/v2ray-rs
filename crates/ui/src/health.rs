use std::collections::VecDeque;
use std::time::{Duration, Instant};

const FAILURE_THRESHOLD: u32 = 3;
const DNS_FAILURE_THRESHOLD: usize = 20;
const DNS_WINDOW: Duration = Duration::from_secs(60);
const DNS_FAILURE_MARKER: &str = "app/dns: failed to retrieve response";

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

#[derive(Debug, Default)]
pub(crate) struct DnsFailureWindow {
    failures: VecDeque<Instant>,
}

impl DnsFailureWindow {
    pub(crate) fn observe(&mut self, line: &str, now: Instant) {
        if !line.contains(DNS_FAILURE_MARKER) {
            return;
        }
        while self
            .failures
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= DNS_WINDOW)
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
    }

    pub(crate) fn is_failing(&self, now: Instant) -> bool {
        let recent = self
            .failures
            .iter()
            .filter(|&&at| now.saturating_duration_since(at) < DNS_WINDOW)
            .count();
        recent >= DNS_FAILURE_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DNS_LINE: &str = "2026/09/16 12:00:00 [Info] app/dns: failed to retrieve response for example.com > context deadline exceeded";

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

    #[test]
    fn dns_window_marks_failing_at_twenty_within_sixty_seconds() {
        let base = Instant::now();
        let mut window = DnsFailureWindow::default();
        for k in 0..19 {
            window.observe(DNS_LINE, base + Duration::from_secs(k * 3));
        }
        let at = base + Duration::from_secs(57);
        assert!(!window.is_failing(at));
        window.observe(DNS_LINE, at);
        assert!(window.is_failing(at));
    }

    #[test]
    fn dns_window_needs_twenty_inside_one_window() {
        let base = Instant::now();
        let mut window = DnsFailureWindow::default();
        for k in 0..20 {
            window.observe(DNS_LINE, base + Duration::from_secs(k * 4));
        }
        assert!(!window.is_failing(base + Duration::from_secs(76)));
    }

    #[test]
    fn dns_window_clears_sixty_seconds_after_the_last_line() {
        let base = Instant::now();
        let mut window = DnsFailureWindow::default();
        for k in 0..20 {
            window.observe(DNS_LINE, base + Duration::from_secs(k));
        }
        let last = base + Duration::from_secs(19);
        assert!(window.is_failing(last));
        assert!(!window.is_failing(last + Duration::from_secs(60)));
    }

    #[test]
    fn dns_window_ignores_tun_noise() {
        let base = Instant::now();
        let mut window = DnsFailureWindow::default();
        let noise = "2026/09/16 12:00:00 [Info] proxy/tun: connection reset by peer";
        for k in 0..500 {
            let now = base + Duration::from_millis(k * 10);
            window.observe(noise, now);
            assert!(!window.is_failing(now));
        }
    }
}
