use thiserror::Error;

use crate::log_buffer::LogLine;
use crate::models::ConnectionMetadata;

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
                | (Starting, Stopping)
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
