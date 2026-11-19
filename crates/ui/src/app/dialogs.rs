use super::*;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum QuitPlan {
    Stop,
    AwaitStopped,
    Release,
    Exit,
}

/// A Disconnect already took the handle while the app is `Stopping`; the
/// window must outlive that stop so its teardown and marker clear finish.
pub(super) fn quit_plan(has_handle: bool, state: &ProcessState, marker_present: bool) -> QuitPlan {
    if has_handle {
        QuitPlan::Stop
    } else if matches!(state, ProcessState::Stopping) {
        QuitPlan::AwaitStopped
    } else if marker_present {
        QuitPlan::Release
    } else {
        QuitPlan::Exit
    }
}

/// A quit signal arriving while a stop is still pending exits at once, so a
/// wedged stop cannot make the app unkillable. UI quits keep waiting.
pub(super) fn force_exit(pending_exit: bool, signal: bool) -> bool {
    pending_exit && signal
}

pub(super) fn quit_line(plan: &QuitPlan) -> String {
    let plan = match plan {
        QuitPlan::Stop => "stop",
        QuitPlan::AwaitStopped => "await-stopped",
        QuitPlan::Release => "release",
        QuitPlan::Exit => "exit",
    };
    format!("quit requested plan={plan}")
}

/// The extra affordance an Error toast can carry.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ToastAction {
    GrantTun,
}

/// The Error toast grows a "Grant TUN privileges" button only when the
/// failed connection is the one whose TUN capability grant was armed.
pub(super) fn error_toast_action(
    generation: u64,
    grant_generation: Option<u64>,
) -> Option<ToastAction> {
    (grant_generation == Some(generation)).then_some(ToastAction::GrantTun)
}

/// Confirmed escape hatch for leftover TUN state the ownership checks refuse
/// to touch: the elevated helper force-removes the device and its routing
/// rules. Runs only on the explicit "recover" response.
pub(super) fn show_recover_tun_dialog(
    window: &adw::ApplicationWindow,
    iface: &str,
    sender: &ComponentSender<App>,
) {
    let sender = sender.clone();
    let dialog = adw::AlertDialog::builder()
        .heading("Recover TUN routes")
        .body(format!(
            "Force-remove the TUN device {iface} and its routing rules? \
             Use this only when no v2ray-rs tunnel is active."
        ))
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("recover", "Recover");
    dialog.set_response_appearance("recover", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "recover" {
            sender.input(AppMsg::RecoverTunOperator);
        }
    });

    dialog.present(Some(window));
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DisconnectPlan {
    Stop,
    Release,
    Nothing,
}

pub(super) fn disconnect_plan(has_handle: bool, marker_present: bool) -> DisconnectPlan {
    if has_handle {
        DisconnectPlan::Stop
    } else if marker_present {
        DisconnectPlan::Release
    } else {
        DisconnectPlan::Nothing
    }
}

pub(super) fn stop_reason_for(
    pending_exit: bool,
    health_failover: bool,
    reconnect_pending: bool,
    direct_target: bool,
) -> StopReason {
    if pending_exit {
        StopReason::AppQuit
    } else if health_failover {
        StopReason::HealthFailover
    } else if reconnect_pending {
        StopReason::ApplyRestart
    } else if direct_target {
        StopReason::NodeSwitch
    } else {
        StopReason::UserStop
    }
}

/// An Error that arrives while the user is disconnecting, or after the
/// automatic reconnect budget is spent, gets no retry, so the kill-switch
/// routes must be released.
pub(super) fn release_on_error(app_state_stopping: bool, reconnects_left: u32) -> bool {
    app_state_stopping || reconnects_left == 0
}
