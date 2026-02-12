use notify_rust::{Notification, Timeout};
use v2ray_rs_process::ProcessState;

const NOTIFICATION_TIMEOUT_MS: u32 = 5000;

#[derive(Clone)]
pub struct Notifier {
    enabled: bool,
}

impl Notifier {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn on_state_change(&self, from: &ProcessState, to: &ProcessState) {
        if !self.enabled {
            return;
        }

        match to {
            ProcessState::Running => {
                self.send("Proxy Connected", "Backend process started successfully");
            }
            ProcessState::Error(msg) => {
                self.send("Proxy Error", msg);
            }
            ProcessState::Stopped if matches!(from, ProcessState::Running) => {
                self.send(
                    "Proxy Disconnected",
                    "Backend process stopped unexpectedly",
                );
            }
            _ => {}
        }
    }

    fn send(&self, summary: &str, body: &str) {
        let _ = Notification::new()
            .appname("V2Ray Manager")
            .summary(summary)
            .body(body)
            .icon("network-vpn")
            .timeout(Timeout::Milliseconds(NOTIFICATION_TIMEOUT_MS))
            .show();
    }
}
