use thiserror::Error;
use tokio::sync::broadcast;

use crate::log_buffer::LogLine;
use v2ray_rs_core::models::ConnectionMetadata;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error(String),
}

impl ProcessState {
    pub fn can_transition_to(&self, target: &ProcessState) -> bool {
        use ProcessState::*;
        matches!(
            (self, target),
            (Stopped, Starting)
                | (Stopped, Error(_))
                | (Starting, Running)
                | (Starting, Error(_))
                // supervised in-place restart after an unexpected exit: the
                // manager relaunches without a user-visible disconnect blip
                | (Running, Starting)
                | (Running, Stopping)
                | (Running, Error(_))
                | (Stopping, Stopped)
                | (Stopping, Error(_))
                | (Error(_), Starting)
                | (Error(_), Stopped)
        )
    }

    pub fn transition(&mut self, target: ProcessState) -> Result<ProcessState, TransitionError> {
        if !self.can_transition_to(&target) {
            return Err(TransitionError::Invalid {
                from: self.clone(),
                to: target,
            });
        }
        let old = std::mem::replace(self, target);
        Ok(old)
    }
}

#[derive(Clone, Debug)]
pub enum ProcessEvent {
    StateChanged {
        from: ProcessState,
        to: ProcessState,
        connection: Option<ConnectionMetadata>,
    },
    LogLine(LogLine),
    ProcessExited {
        exit_code: Option<i32>,
    },
}

#[derive(Debug, Error)]
pub enum TransitionError {
    #[error("invalid state transition from {from:?} to {to:?}")]
    Invalid {
        from: ProcessState,
        to: ProcessState,
    },
}

pub struct StateManager {
    state: ProcessState,
    tx: broadcast::Sender<ProcessEvent>,
    log_tx: broadcast::Sender<ProcessEvent>,
}

impl StateManager {
    pub fn new() -> Self {
        // Logs get their own high-capacity channel so a burst of backend output
        // can never lag the state channel and drop a terminal StateChanged.
        let (tx, _rx) = broadcast::channel(64);
        let (log_tx, _log_rx) = broadcast::channel(1024);
        Self {
            state: ProcessState::Stopped,
            tx,
            log_tx,
        }
    }

    pub fn state(&self) -> ProcessState {
        self.state.clone()
    }

    pub fn transition(
        &mut self,
        target: ProcessState,
        connection: Option<ConnectionMetadata>,
    ) -> Result<ProcessState, TransitionError> {
        let old = self.state.transition(target.clone())?;
        let _ = self.tx.send(ProcessEvent::StateChanged {
            from: old.clone(),
            to: target,
            connection,
        });
        Ok(old)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<ProcessEvent> {
        self.log_tx.subscribe()
    }

    pub fn log_sender(&self) -> &broadcast::Sender<ProcessEvent> {
        &self.log_tx
    }

    pub fn emit(&self, event: ProcessEvent) {
        let tx = match event {
            ProcessEvent::LogLine(_) => &self.log_tx,
            _ => &self.tx,
        };
        let _ = tx.send(event);
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions_succeed() {
        let mut state = ProcessState::Stopped;
        assert!(state.transition(ProcessState::Starting).is_ok());
        assert_eq!(state, ProcessState::Starting);

        assert!(state.transition(ProcessState::Running).is_ok());
        assert_eq!(state, ProcessState::Running);

        assert!(state.transition(ProcessState::Stopping).is_ok());
        assert_eq!(state, ProcessState::Stopping);

        assert!(state.transition(ProcessState::Stopped).is_ok());
        assert_eq!(state, ProcessState::Stopped);
    }

    #[test]
    fn error_transitions() {
        let mut state = ProcessState::Stopped;
        assert!(
            state
                .transition(ProcessState::Error("preflight".into()))
                .is_ok()
        );
        assert_eq!(state, ProcessState::Error("preflight".into()));

        let mut state = ProcessState::Starting;
        assert!(state.transition(ProcessState::Error("test".into())).is_ok());
        assert_eq!(state, ProcessState::Error("test".into()));

        assert!(state.transition(ProcessState::Starting).is_ok());
        assert_eq!(state, ProcessState::Starting);

        state = ProcessState::Error("test".into());
        assert!(state.transition(ProcessState::Stopped).is_ok());
        assert_eq!(state, ProcessState::Stopped);
    }

    #[test]
    fn invalid_transitions_fail() {
        let mut state = ProcessState::Stopped;
        assert!(state.transition(ProcessState::Running).is_err());

        state = ProcessState::Running;
        assert!(state.transition(ProcessState::Stopped).is_err());

        state = ProcessState::Stopped;
        assert!(state.transition(ProcessState::Stopping).is_err());
    }

    #[test]
    fn state_manager_broadcasts_events() {
        let mut mgr = StateManager::new();
        let mut rx = mgr.subscribe();

        mgr.transition(ProcessState::Starting, None).unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            ProcessEvent::StateChanged { from, to, .. } => {
                assert_eq!(from, ProcessState::Stopped);
                assert_eq!(to, ProcessState::Starting);
            }
            _ => panic!("expected StateChanged"),
        }
    }

    #[test]
    fn multiple_subscribers_receive_events() {
        let mut mgr = StateManager::new();
        let mut rx1 = mgr.subscribe();
        let mut rx2 = mgr.subscribe();

        mgr.transition(ProcessState::Starting, None).unwrap();

        let event1 = rx1.try_recv().unwrap();
        let event2 = rx2.try_recv().unwrap();

        match (event1, event2) {
            (
                ProcessEvent::StateChanged {
                    from: f1, to: t1, ..
                },
                ProcessEvent::StateChanged {
                    from: f2, to: t2, ..
                },
            ) => {
                assert_eq!(f1, ProcessState::Stopped);
                assert_eq!(t1, ProcessState::Starting);
                assert_eq!(f2, ProcessState::Stopped);
                assert_eq!(t2, ProcessState::Starting);
            }
            _ => panic!("expected StateChanged"),
        }
    }

    #[test]
    fn state_manager_emit() {
        let mgr = StateManager::new();
        let mut rx = mgr.subscribe_logs();

        mgr.emit(ProcessEvent::LogLine(LogLine::stdout("test")));

        let event = rx.try_recv().unwrap();
        match event {
            ProcessEvent::LogLine(line) => {
                assert_eq!(line.content, "test");
            }
            _ => panic!("expected LogLine"),
        }
    }

    #[test]
    fn state_manager_starts_stopped() {
        let mgr = StateManager::new();
        assert_eq!(mgr.state(), ProcessState::Stopped);
    }

    #[test]
    fn transition_returns_old_state() {
        let mut mgr = StateManager::new();
        let old = mgr.transition(ProcessState::Starting, None).unwrap();
        assert_eq!(old, ProcessState::Stopped);
        assert_eq!(mgr.state(), ProcessState::Starting);
    }
}
