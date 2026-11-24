use std::sync::Arc;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Handle, Tray, TrayMethods};
use tokio::sync::watch;
use v2ray_rs_core::models::ConnectionMetadata;
use v2ray_rs_core::state::ProcessState;

use crate::icons;
use crate::notification::Notifier;

/// One authoritative tray view, published by the UI on every applied state
/// and cleanup-flag change. It drives the menu and icon only: a `watch`
/// coalesces missed intermediates by design, so nothing may depend on the
/// tray observing every transition — desktop transition notifications are
/// produced in the app on accepted `apply_state` transitions and delivered
/// through `TrayHandle::notify_state_change`.
#[derive(Debug, Clone)]
pub struct TraySnapshot {
    pub state: ProcessState,
    pub connection: Option<ConnectionMetadata>,
    pub cleanup_pending: bool,
}

impl TraySnapshot {
    pub fn initial() -> Self {
        Self {
            state: ProcessState::Stopped,
            connection: None,
            cleanup_pending: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrayAction {
    Connect,
    Disconnect,
    ShowWindow,
    Quit,
}

pub struct TrayHandle {
    handle: Handle<AppTray>,
    listener: tokio::task::JoinHandle<()>,
    notifier: Notifier,
}

impl TrayHandle {
    pub async fn shutdown(&self) {
        self.listener.abort();
        self.handle.shutdown().await;
    }

    pub fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifier.set_enabled(enabled);
    }

    pub fn notify(&self, summary: &str, body: &str) {
        let notifier = self.notifier.clone();
        let (summary, body) = (summary.to_owned(), body.to_owned());
        tokio::task::spawn_blocking(move || notifier.notify(&summary, &body));
    }

    /// Posts the desktop state-transition notification. The producer
    /// compares consecutive accepted states and decides whether a
    /// transition happened; this path only delivers, honoring the
    /// notification setting like every notifier path.
    pub fn notify_state_change(&self, from: &ProcessState, to: &ProcessState) {
        let notifier = self.notifier.clone();
        let (from, to) = (from.clone(), to.clone());
        tokio::task::spawn_blocking(move || notifier.on_state_change(&from, &to));
    }
}

struct AppTray {
    tray_id: String,
    process_state: ProcessState,
    connection: Option<ConnectionMetadata>,
    cleanup_pending: bool,
    on_action: Arc<dyn Fn(TrayAction) + Send + Sync>,
}

impl Tray for AppTray {
    fn id(&self) -> String {
        self.tray_id.clone()
    }

    fn title(&self) -> String {
        "V2Ray Manager".into()
    }

    fn icon_name(&self) -> String {
        // Some hosts ignore IconThemePath; use pixmap for consistent rendering.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        match &self.process_state {
            ProcessState::Running => icons::connected_pixmap(),
            ProcessState::Error(_) => icons::error_pixmap(),
            _ => icons::disconnected_pixmap(),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let (title, description) = match (&self.process_state, &self.connection) {
            (ProcessState::Running, Some(meta)) => {
                ("Connected".to_string(), tooltip_description(meta))
            }
            (ProcessState::Starting, _) => (
                "Connecting".to_string(),
                "Resolving nodes and starting backend".to_string(),
            ),
            (ProcessState::Stopping, _) => {
                ("Disconnecting".to_string(), "Stopping backend".to_string())
            }
            (ProcessState::Error(msg), _) => ("Error".to_string(), msg.clone()),
            _ => (
                "Disconnected".to_string(),
                "No active connection".to_string(),
            ),
        };

        ksni::ToolTip {
            title,
            description,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let toggle = {
            let (action, enabled) = toggle_action(&self.process_state, self.cleanup_pending);
            let label = match action {
                TrayAction::Disconnect => "Disconnect",
                _ => "Connect",
            };
            let cb = Arc::clone(&self.on_action);
            StandardItem {
                label: label.into(),
                enabled,
                activate: Box::new(move |_| {
                    (cb)(action.clone());
                }),
                ..Default::default()
            }
        };

        let status_label = match &self.process_state {
            ProcessState::Stopped => "Status: Disconnected".to_string(),
            ProcessState::Starting => "Status: Connecting...".to_string(),
            ProcessState::Running => {
                if let Some(meta) = &self.connection {
                    format!("Status: Connected ({})", connected_source_and_node(meta))
                } else {
                    "Status: Connected".to_string()
                }
            }
            ProcessState::Stopping => "Status: Disconnecting...".to_string(),
            ProcessState::Error(msg) => return self.menu_with_error(toggle, msg),
        };

        let show_window = {
            let cb = Arc::clone(&self.on_action);
            StandardItem {
                label: "Open Main Window".into(),
                activate: Box::new(move |_| {
                    (cb)(TrayAction::ShowWindow);
                }),
                ..Default::default()
            }
        };

        let quit = {
            let cb = Arc::clone(&self.on_action);
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    (cb)(TrayAction::Quit);
                }),
                ..Default::default()
            }
        };

        vec![
            toggle.into(),
            MenuItem::Separator,
            StandardItem {
                label: status_label,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            show_window.into(),
            quit.into(),
        ]
    }
}

/// `Starting` offers Disconnect so a slow connect can be cancelled;
/// `Stopping` has nothing left to cancel. A pending cleanup blocks every
/// connect: the recovery control in the window owns the outcome.
fn toggle_action(state: &ProcessState, cleanup_pending: bool) -> (TrayAction, bool) {
    let (action, enabled) = match state {
        ProcessState::Running | ProcessState::Starting => (TrayAction::Disconnect, true),
        ProcessState::Stopping => (TrayAction::Connect, false),
        ProcessState::Stopped | ProcessState::Error(_) => (TrayAction::Connect, true),
    };
    if cleanup_pending && matches!(action, TrayAction::Connect) {
        return (action, false);
    }
    (action, enabled)
}

fn connected_source_and_node(meta: &ConnectionMetadata) -> String {
    format!("{} · {}", meta.source, meta.node_name)
}

fn tooltip_description(meta: &ConnectionMetadata) -> String {
    let latency = meta
        .latency_ms
        .map(|ms| format!("{ms} ms"))
        .unwrap_or_else(|| "n/a".into());
    format!(
        "{}\n{}\nLatency: {}\nBackend: {}\nStrategy: {}\nSince: {}",
        meta.source,
        meta.node_name,
        latency,
        meta.backend,
        meta.strategy,
        meta.connected_since.format("%Y-%m-%d %H:%M")
    )
}

impl AppTray {
    fn menu_with_error(&self, toggle: StandardItem<Self>, msg: &str) -> Vec<MenuItem<Self>> {
        let show_window = {
            let cb = Arc::clone(&self.on_action);
            StandardItem {
                label: "Open Main Window".into(),
                activate: Box::new(move |_| {
                    (cb)(TrayAction::ShowWindow);
                }),
                ..Default::default()
            }
        };

        let quit = {
            let cb = Arc::clone(&self.on_action);
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    (cb)(TrayAction::Quit);
                }),
                ..Default::default()
            }
        };

        let truncated = if msg.len() > 50 {
            let boundary = msg
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= 47)
                .last()
                .unwrap_or(0);
            format!("Error: {}...", &msg[..boundary])
        } else {
            format!("Error: {msg}")
        };

        vec![
            toggle.into(),
            MenuItem::Separator,
            StandardItem {
                label: truncated,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            show_window.into(),
            quit.into(),
        ]
    }
}

/// Applies one snapshot to the menu and icon. The tray is a pure view of
/// the latest published snapshot and never announces transitions: the
/// watch coalesces intermediates by design, so firing the notifier here
/// would drop real transitions and invent others.
async fn apply_snapshot(handle: &Handle<AppTray>, snapshot: &TraySnapshot) {
    let state = snapshot.state.clone();
    let connection = snapshot.connection.clone();
    let cleanup_pending = snapshot.cleanup_pending;
    handle
        .update(move |tray| {
            tray.process_state = state;
            tray.connection = connection;
            tray.cleanup_pending = cleanup_pending;
        })
        .await;
}

/// Converges on the latest published snapshot for the menu and icon. The
/// watch coalesces missed intermediates by design, so the listener never
/// replays history and never fires notifications: transition notices are
/// the producer's job.
fn spawn_listener(
    mut snapshot_rx: watch::Receiver<TraySnapshot>,
    handle: Handle<AppTray>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while snapshot_rx.changed().await.is_ok() {
            let snapshot = snapshot_rx.borrow_and_update().clone();
            apply_snapshot(&handle, &snapshot).await;
        }
    })
}

pub struct TrayService;

impl TrayService {
    pub async fn spawn(
        snapshot_rx: watch::Receiver<TraySnapshot>,
        notifier: Notifier,
        on_action: impl Fn(TrayAction) + Send + Sync + 'static,
        tray_id: String,
    ) -> Result<TrayHandle, ksni::Error> {
        let on_action = Arc::new(on_action);

        let tray = AppTray {
            tray_id,
            process_state: ProcessState::Stopped,
            connection: None,
            cleanup_pending: false,
            on_action,
        };

        let handle = tray.spawn().await?;
        let listener = spawn_listener(snapshot_rx, handle.clone());

        Ok(TrayHandle {
            handle,
            listener,
            notifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use chrono::Utc;
    use uuid::Uuid;
    use v2ray_rs_core::models::{AutoResolveStrategy, BackendType, ConnectionNodeRef};

    use super::*;

    /// The parent test re-runs this binary through `dbus-run-session`; the
    /// child inherits the isolated bus address, so the tray never registers on
    /// the developer's desktop bus.
    const DBUS_CHILD_ENV: &str = "V2RAY_RS_TRAY_DBUS_CHILD";
    const LIVE_TEST_NAME: &str = "tray::tests::publishes_rapid_snapshots_to_an_isolated_bus";

    fn sample_metadata() -> ConnectionMetadata {
        ConnectionMetadata {
            node_ref: ConnectionNodeRef::Manual {
                node_id: Uuid::nil(),
            },
            source: "Manual".into(),
            source_id: Uuid::nil().to_string(),
            node_name: "Test Node".into(),
            node_address: "example.com".into(),
            node_port: 443,
            backend: BackendType::Xray,
            strategy: AutoResolveStrategy::ListOrder,
            latency_ms: Some(42),
            connected_since: Utc::now(),
        }
    }

    #[test]
    fn connected_status_text_includes_source_and_node() {
        let meta = sample_metadata();
        assert_eq!(connected_source_and_node(&meta), "Manual · Test Node");
    }

    #[test]
    fn tooltip_description_includes_manual_source() {
        let meta = sample_metadata();
        let description = tooltip_description(&meta);
        assert!(description.starts_with("Manual\nTest Node"));
        assert!(description.contains("Latency: 42 ms"));
    }

    #[test]
    fn toggle_is_enabled_disconnect_while_starting() {
        assert!(matches!(
            toggle_action(&ProcessState::Starting, false),
            (TrayAction::Disconnect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Running, false),
            (TrayAction::Disconnect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Stopped, false),
            (TrayAction::Connect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Error("boom".into()), false),
            (TrayAction::Connect, true)
        ));
    }

    #[test]
    fn toggle_disabled_while_stopping() {
        let (_, enabled) = toggle_action(&ProcessState::Stopping, false);
        assert!(!enabled);
    }

    #[test]
    fn cleanup_pending_disables_connect_only() {
        assert!(matches!(
            toggle_action(&ProcessState::Stopped, true),
            (TrayAction::Connect, false)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Error("boom".into()), true),
            (TrayAction::Connect, false)
        ));
        // A live session still offers Disconnect while its cleanup reports.
        assert!(matches!(
            toggle_action(&ProcessState::Running, true),
            (TrayAction::Disconnect, true)
        ));
    }

    #[test]
    fn initial_snapshot_is_stopped_without_cleanup() {
        let snapshot = TraySnapshot::initial();
        assert_eq!(snapshot.state, ProcessState::Stopped);
        assert!(snapshot.connection.is_none());
        assert!(!snapshot.cleanup_pending);
    }

    #[test]
    fn cleanup_pending_menu_keeps_open_and_quit_but_blocks_connect() {
        let tray = AppTray {
            tray_id: "com.github.v2ray-rs.dev".into(),
            process_state: ProcessState::Stopped,
            connection: None,
            cleanup_pending: true,
            on_action: Arc::new(|_action: TrayAction| {}),
        };
        let menu = Tray::menu(&tray);
        let toggle = menu.iter().find_map(|item| match item {
            MenuItem::Standard(item) if item.label == "Connect" || item.label == "Disconnect" => {
                Some(item)
            }
            _ => None,
        });
        let toggle = toggle.expect("the menu offers the connect toggle");
        assert_eq!(toggle.label, "Connect");
        assert!(!toggle.enabled, "cleanup pending must block Connect");
        for expected in ["Open Main Window", "Quit"] {
            let item = menu
                .iter()
                .find_map(|item| match item {
                    MenuItem::Standard(item) if item.label == expected => Some(item),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the menu keeps {expected}"));
            assert!(item.enabled, "{expected} must stay usable");
        }
    }

    #[test]
    fn id_returns_injected_tray_id() {
        let tray = AppTray {
            tray_id: "com.github.v2ray-rs.dev".into(),
            process_state: ProcessState::Stopped,
            connection: None,
            cleanup_pending: false,
            on_action: Arc::new(|_action: TrayAction| {}),
        };
        assert_eq!(Tray::id(&tray), "com.github.v2ray-rs.dev");
    }

    /// A live StatusNotifierItem needs a session bus, and it must never touch
    /// the developer's desktop session. This parent starts a private
    /// `dbus-run-session` and re-runs the binary there, so the tray under test
    /// only ever sees an isolated bus.
    #[test]
    fn live_tray_requires_an_isolated_session_bus() {
        if std::env::var_os(DBUS_CHILD_ENV).is_some() {
            return;
        }
        let missing: Vec<&str> = ["dbus-run-session", "gdbus"]
            .into_iter()
            .filter(|tool| !tool_on_path(tool))
            .collect();
        if !missing.is_empty() {
            let reason = format!("missing {}", missing.join(", "));
            assert!(
                std::env::var_os("V2RAY_RS_REQUIRE_DBUS").is_none(),
                "required tray D-Bus prerequisite unavailable: {reason}"
            );
            eprintln!("skipping live tray test: {reason}");
            return;
        }

        let exe = std::env::current_exe().expect("test binary path");
        let mut child = Command::new("dbus-run-session")
            .arg("--")
            .arg(&exe)
            .args(["--exact", LIVE_TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(DBUS_CHILD_ENV, "1")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start dbus-run-session");
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    assert!(status.success(), "live tray test failed: {status}");
                    return;
                }
                Ok(None) => {}
                Err(err) => panic!("while waiting for the live tray test: {err}"),
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("live tray test exceeded 90s");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Drives the real `spawn_listener` against a real StatusNotifierItem on
    /// the isolated bus. The menu and icon must converge on the latest
    /// published snapshot even when the watch coalesces a rapid burst; a unit
    /// assertion on `AppTray::menu()` would not prove that.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publishes_rapid_snapshots_to_an_isolated_bus() {
        if std::env::var_os(DBUS_CHILD_ENV).is_none() {
            return;
        }

        let (snapshot_tx, snapshot_rx) = watch::channel(TraySnapshot::initial());
        let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel();
        let tray = AppTray {
            tray_id: format!("org.v2ray-rs.tray-test.{}", std::process::id()),
            process_state: ProcessState::Stopped,
            connection: None,
            cleanup_pending: false,
            on_action: Arc::new(move |action: TrayAction| {
                let _ = action_tx.send(action);
            }),
        };
        // A bare session bus has no StatusNotifierWatcher; ksni treats the
        // missing watcher as a soft error only when the host is assumed.
        let handle = tray
            .assume_sni_available(true)
            .spawn()
            .await
            .expect("spawn the tray item on the isolated session bus");
        let listener = spawn_listener(snapshot_rx, handle.clone());

        let item = wait_for_bus_item().await;

        // Positive control: the item answers on the bus, carries our identity,
        // and exposes the disconnected menu before any snapshot is published.
        let identity = item_property(&item, "Id").await;
        assert!(
            identity.contains("org.v2ray-rs.tray-test."),
            "unexpected StatusNotifierItem id: {identity}"
        );
        let initial_icon = item_property(&item, "IconPixmap").await;
        assert!(
            initial_icon.contains("(22, 22,"),
            "unexpected icon payload: {initial_icon}"
        );
        let initial = await_menu(&item, Duration::from_secs(10), |items| {
            let toggle = toggle(items);
            toggle.label == "Connect" && toggle.enabled
        })
        .await;
        assert_eq!(status_label(&initial), "Status: Disconnected");
        assert!(menu_item(&initial, "Open Main Window").enabled);
        assert!(menu_item(&initial, "Quit").enabled);

        // Rapid burst: only the final snapshot may drive the published menu.
        for step in 0..24 {
            let snapshot = match step % 4 {
                0 => TraySnapshot {
                    state: ProcessState::Starting,
                    connection: None,
                    cleanup_pending: false,
                },
                1 => running_snapshot(),
                2 => TraySnapshot {
                    state: ProcessState::Stopping,
                    connection: None,
                    cleanup_pending: false,
                },
                _ => TraySnapshot {
                    state: ProcessState::Error("engine lost".into()),
                    connection: None,
                    cleanup_pending: false,
                },
            };
            snapshot_tx
                .send(snapshot)
                .expect("tray listener stays alive");
        }
        snapshot_tx
            .send(TraySnapshot {
                state: ProcessState::Stopped,
                connection: None,
                cleanup_pending: true,
            })
            .expect("tray listener stays alive");

        let pending = await_menu(&item, Duration::from_secs(10), |items| {
            let toggle = toggle(items);
            toggle.label == "Connect"
                && !toggle.enabled
                && status_label(items) == "Status: Disconnected"
        })
        .await;
        assert!(
            menu_item(&pending, "Open Main Window").enabled,
            "Open Main Window must stay usable: {pending:?}"
        );
        assert!(
            menu_item(&pending, "Quit").enabled,
            "Quit must stay usable: {pending:?}"
        );

        // Retained items stay wired to the action callback at the D-Bus level.
        click(&item, menu_item(&pending, "Quit").id).await;
        assert!(
            matches!(
                tokio::time::timeout(Duration::from_secs(5), action_rx.recv()).await,
                Ok(Some(TrayAction::Quit))
            ),
            "Quit must stay wired while cleanup is pending"
        );
        click(&item, menu_item(&pending, "Open Main Window").id).await;
        assert!(
            matches!(
                tokio::time::timeout(Duration::from_secs(5), action_rx.recv()).await,
                Ok(Some(TrayAction::ShowWindow))
            ),
            "Open Main Window must stay wired while cleanup is pending"
        );

        // A second burst ends connected: menu, icon and tooltip all follow the
        // latest snapshot instead of replaying an intermediate one.
        for step in 0..24 {
            let snapshot = match step % 3 {
                0 => TraySnapshot::initial(),
                1 => TraySnapshot {
                    state: ProcessState::Starting,
                    connection: None,
                    cleanup_pending: false,
                },
                _ => running_snapshot(),
            };
            snapshot_tx
                .send(snapshot)
                .expect("tray listener stays alive");
        }
        snapshot_tx
            .send(running_snapshot())
            .expect("tray listener stays alive");

        let connected = await_menu(&item, Duration::from_secs(10), |items| {
            let toggle = toggle(items);
            toggle.label == "Disconnect" && toggle.enabled
        })
        .await;
        assert_eq!(
            status_label(&connected),
            "Status: Connected (Manual · Test Node)"
        );
        let connected_icon = item_property(&item, "IconPixmap").await;
        assert_ne!(
            connected_icon, initial_icon,
            "the icon must follow the snapshot"
        );
        let tooltip = item_property(&item, "ToolTip").await;
        assert!(
            tooltip.contains("Connected"),
            "tooltip follows the snapshot: {tooltip}"
        );

        // A third burst returns to disconnected: the latest menu and icon are
        // restored exactly, with Connect usable again.
        for step in 0..24 {
            let snapshot = match step % 3 {
                0 => running_snapshot(),
                1 => TraySnapshot {
                    state: ProcessState::Stopping,
                    connection: None,
                    cleanup_pending: false,
                },
                _ => TraySnapshot::initial(),
            };
            snapshot_tx
                .send(snapshot)
                .expect("tray listener stays alive");
        }
        snapshot_tx
            .send(TraySnapshot::initial())
            .expect("tray listener stays alive");

        let restored = await_menu(&item, Duration::from_secs(10), |items| {
            let toggle = toggle(items);
            toggle.label == "Connect" && toggle.enabled
        })
        .await;
        assert_eq!(status_label(&restored), "Status: Disconnected");
        assert!(menu_item(&restored, "Open Main Window").enabled);
        assert!(menu_item(&restored, "Quit").enabled);
        let restored_icon = item_property(&item, "IconPixmap").await;
        assert_eq!(
            restored_icon, initial_icon,
            "the latest snapshot restores the disconnected icon"
        );

        listener.abort();
        handle.shutdown().await;
    }

    fn running_snapshot() -> TraySnapshot {
        TraySnapshot {
            state: ProcessState::Running,
            connection: Some(sample_metadata()),
            cleanup_pending: false,
        }
    }

    fn tool_on_path(tool: &str) -> bool {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
            .unwrap_or(false)
    }

    #[derive(Debug, Clone)]
    struct LiveMenuItem {
        id: i32,
        label: String,
        enabled: bool,
    }

    /// `com.canonical.dbusmenu.GetLayout` arrives as GVariant text; only the
    /// item dicts are needed, so dicts are split on their delimiters and read
    /// key by key. Dictionary order is not stable across processes.
    fn parse_layout(layout: &str) -> Vec<LiveMenuItem> {
        let mut items = Vec::new();
        let mut rest = layout;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else {
                break;
            };
            let properties = &after[..close];
            if let Some(label) = quoted_after(properties, "'label': <'") {
                items.push(LiveMenuItem {
                    id: trailing_id(&rest[..open]).unwrap_or(-1),
                    label,
                    // dbusmenu omits properties equal to their default, and
                    // `enabled` defaults to true.
                    enabled: !properties.contains("'enabled': <false>"),
                });
            }
            rest = &after[close + 1..];
        }
        items
    }

    fn quoted_after(text: &str, marker: &str) -> Option<String> {
        let start = text.find(marker)? + marker.len();
        let end = text[start..].find('\'')? + start;
        Some(text[start..end].to_string())
    }

    fn trailing_id(prefix: &str) -> Option<i32> {
        let digits: String = prefix
            .trim_end()
            .strip_suffix(',')?
            .trim_end()
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        digits.parse().ok()
    }

    fn toggle(items: &[LiveMenuItem]) -> &LiveMenuItem {
        let mut found = items
            .iter()
            .filter(|item| matches!(item.label.as_str(), "Connect" | "Disconnect"));
        let first = found
            .next()
            .unwrap_or_else(|| panic!("menu has the connect toggle: {items:?}"));
        assert!(
            found.next().is_none(),
            "menu has exactly one connect toggle: {items:?}"
        );
        first
    }

    fn menu_item<'a>(items: &'a [LiveMenuItem], label: &str) -> &'a LiveMenuItem {
        items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("menu has {label}: {items:?}"))
    }

    fn status_label(items: &[LiveMenuItem]) -> &str {
        items
            .iter()
            .find(|item| item.label.starts_with("Status: "))
            .unwrap_or_else(|| panic!("menu has a status label: {items:?}"))
            .label
            .as_str()
    }

    async fn wait_for_bus_item() -> String {
        let prefix = format!("org.kde.StatusNotifierItem-{}-", std::process::id());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let names = call_gdbus(vec![
                "--dest".into(),
                "org.freedesktop.DBus".into(),
                "--object-path".into(),
                "/org/freedesktop/DBus".into(),
                "--method".into(),
                "org.freedesktop.DBus.ListNames".into(),
            ])
            .await;
            if let Some(name) = quoted_strings(&names).find(|name| name.starts_with(&prefix)) {
                return name;
            }
            if Instant::now() >= deadline {
                panic!("tray item never appeared on the isolated bus: {names}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn quoted_strings(text: &str) -> impl Iterator<Item = String> + '_ {
        text.split('\'').skip(1).step_by(2).map(str::to_string)
    }

    async fn item_property(item: &str, property: &str) -> String {
        call_gdbus(vec![
            "--dest".into(),
            item.into(),
            "--object-path".into(),
            "/StatusNotifierItem".into(),
            "--method".into(),
            "org.freedesktop.DBus.Properties.Get".into(),
            "org.kde.StatusNotifierItem".into(),
            property.into(),
        ])
        .await
    }

    async fn await_menu(
        item: &str,
        within: Duration,
        mut converged: impl FnMut(&[LiveMenuItem]) -> bool,
    ) -> Vec<LiveMenuItem> {
        let deadline = Instant::now() + within;
        loop {
            let layout = call_gdbus(vec![
                "--dest".into(),
                item.into(),
                "--object-path".into(),
                "/MenuBar".into(),
                "--method".into(),
                "com.canonical.dbusmenu.GetLayout".into(),
                "0".into(),
                // The menu is flat: two levels cover the root and its items.
                // gdbus rejects `-1` here because it parses as an option.
                "2".into(),
                "['label', 'enabled']".into(),
            ])
            .await;
            let items = parse_layout(&layout);
            if converged(&items) {
                return items;
            }
            if Instant::now() >= deadline {
                panic!("menu did not converge within {within:?}: {layout}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn click(item: &str, id: i32) -> String {
        let id = id.to_string();
        call_gdbus(vec![
            "--dest".into(),
            item.into(),
            "--object-path".into(),
            "/MenuBar".into(),
            "--method".into(),
            "com.canonical.dbusmenu.Event".into(),
            id,
            "clicked".into(),
            "<0>".into(),
            "0".into(),
        ])
        .await
    }

    async fn call_gdbus(args: Vec<String>) -> String {
        tokio::task::spawn_blocking(move || gdbus_call(&args))
            .await
            .expect("gdbus worker panicked")
    }

    fn gdbus_call(args: &[String]) -> String {
        let mut child = Command::new("gdbus")
            .arg("call")
            .arg("--session")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start gdbus");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait().expect("poll gdbus") {
                Some(_) => {
                    let output = child.wait_with_output().expect("read gdbus output");
                    assert!(
                        output.status.success(),
                        "gdbus {} failed: {}",
                        args.join(" "),
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    return String::from_utf8_lossy(&output.stdout).into_owned();
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("gdbus {} exceeded 5s", args.join(" "));
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}
