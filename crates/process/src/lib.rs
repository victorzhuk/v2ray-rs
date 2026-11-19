mod manager;
mod pid;
mod privilege;
mod probe;
pub mod session;
mod spawn;
mod state;
mod tun;

#[cfg(any(test, feature = "test-utils"))]
pub use manager::HostProbe;
pub use manager::{ProcessError, ProcessManager, StopReason};
pub use pid::PidFile;
pub use privilege::{
    BACKEND_CAPS, HELPER_CAPS, PrivilegeError, ProbeFailure, grant, has_net_admin, manual_command,
};
pub use probe::{ProbeError, ProbeRunner};
pub use state::{ProcessEvent, ProcessState, TransitionError};
pub use tun::{
    BYPASS_USER, HELPER_TIMEOUT, HelperAccess, HelperRun, TunJournal, TunRuntime,
    connected_subnets, helper_access, helper_path, helpers_stale, host_has_ipv6, operator_recover,
    operator_recover_args, relocated_helper_path, relocation_required, run_helper, run_path,
    verify_singbox_stopped,
};
pub use v2ray_rs_core::log_buffer::{LogBuffer, LogLine, LogSource};
