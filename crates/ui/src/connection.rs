use v2ray_rs_process::session::{EventSender, SessionEvent};

use crate::app::AppMsg;

pub use v2ray_rs_process::session::{
    HealthTiming, SessionHandle as ConnectionHandle, SessionRequest as ConnectionRequest,
    TunLifecycle, v2ray_tun_warning,
};

/// Starts a session supervisor and forwards its events into the Relm4 message
/// loop. All connection logic lives in `v2ray_rs_process::session`; this module
/// only translates the transport.
pub fn spawn(request: ConnectionRequest, sender: relm4::Sender<AppMsg>) -> ConnectionHandle {
    let (events, mut events_rx) = EventSender::channel();
    let handle = session_spawn(request, events);

    tokio::spawn(async move {
        while let Some(event) = events_rx.recv().await {
            sender.emit(to_app_msg(event));
        }
    });

    handle
}

#[cfg(not(test))]
fn session_spawn(request: ConnectionRequest, events: EventSender) -> ConnectionHandle {
    v2ray_rs_process::session::spawn(request, events)
}

#[cfg(test)]
type TestConfigure = Box<
    dyn Fn(v2ray_rs_process::ProcessManager) -> v2ray_rs_process::ProcessManager + Send + 'static,
>;

#[cfg(test)]
thread_local! {
    static TEST_CONFIGURE: std::cell::RefCell<Option<TestConfigure>> =
        const { std::cell::RefCell::new(None) };
}

/// Installs a one-shot manager hook for the next [`spawn`] on this thread.
/// Joined GTK tests use it to pin host probes and a helper stub; production
/// builds contain no hook at all.
#[cfg(test)]
pub(crate) fn inject_test_configure(configure: TestConfigure) {
    TEST_CONFIGURE.with(|slot| *slot.borrow_mut() = Some(configure));
}

#[cfg(test)]
fn session_spawn(request: ConnectionRequest, events: EventSender) -> ConnectionHandle {
    match TEST_CONFIGURE.with(|slot| slot.borrow_mut().take()) {
        Some(configure) => {
            v2ray_rs_process::session::spawn_configured(request, events, configure, |_| {
                Ok(Vec::new())
            })
        }
        None => v2ray_rs_process::session::spawn(request, events),
    }
}

fn to_app_msg(event: SessionEvent) -> AppMsg {
    match event {
        SessionEvent::State(generation, state, connection) => {
            AppMsg::ProcessStateConnection(generation, state, connection)
        }
        SessionEvent::LogLines(generation, lines) => AppMsg::ProcessLogLines(generation, lines),
        SessionEvent::Toast(text) => AppMsg::ShowToast(text),
        SessionEvent::Health(generation, health) => AppMsg::ConnectionHealth(generation, health),
        SessionEvent::DnsHealth(generation, failing) => AppMsg::DnsHealth(generation, failing),
        SessionEvent::TunGrantRequired(generation) => AppMsg::TunGrantRequired(generation),
        SessionEvent::CleanupPending(generation, message) => {
            AppMsg::CleanupPending(generation, message)
        }
        SessionEvent::CleanupRecoveryRequired(generation, message) => {
            AppMsg::CleanupRecoveryRequired(generation, message)
        }
        SessionEvent::EngineDegraded { generation, detail } => {
            AppMsg::EngineDegraded(generation, detail)
        }
        SessionEvent::NodePins(generation, pins) => AppMsg::NodePins(generation, pins),
        SessionEvent::OfferSubscriptionRefresh {
            subscription_id,
            message,
        } => AppMsg::OfferSubscriptionRefresh {
            subscription_id,
            message,
        },
    }
}
