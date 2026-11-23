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
    let handle = v2ray_rs_process::session::spawn(request, events);

    tokio::spawn(async move {
        while let Some(event) = events_rx.recv().await {
            sender.emit(to_app_msg(event));
        }
    });

    handle
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
