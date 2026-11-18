use std::path::Path;
use std::sync::Arc;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Handle, Tray, TrayMethods};
use tempfile::TempDir;
use tokio::sync::broadcast;
use v2ray_rs_core::models::ConnectionMetadata;
use v2ray_rs_core::state::{ProcessEvent, ProcessState};

use crate::icons;
use crate::notification::Notifier;

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
}

struct AppTray {
    tray_id: String,
    process_state: ProcessState,
    connection: Option<ConnectionMetadata>,
    on_action: Arc<dyn Fn(TrayAction) + Send + Sync>,
    _icon_theme_dir: Option<TempDir>,
    icon_theme_path: String,
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

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
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
            let (action, enabled) = toggle_action(&self.process_state);
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
/// `Stopping` has nothing left to cancel.
fn toggle_action(state: &ProcessState) -> (TrayAction, bool) {
    match state {
        ProcessState::Running | ProcessState::Starting => (TrayAction::Disconnect, true),
        ProcessState::Stopping => (TrayAction::Connect, false),
        ProcessState::Stopped | ProcessState::Error(_) => (TrayAction::Connect, true),
    }
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

/// Applies one tray state update and fires the matching notification.
async fn apply_state_event(
    handle: &Handle<AppTray>,
    notifier: &Notifier,
    from: ProcessState,
    to: ProcessState,
    connection: Option<ConnectionMetadata>,
) {
    let state = to.clone();
    let update_connection = connection.clone();
    handle
        .update(move |tray| {
            tray.process_state = state;
            tray.connection = update_connection;
        })
        .await;
    let n = notifier.clone();
    tokio::task::spawn_blocking(move || {
        n.on_state_change(&from, &to);
    });
}

fn spawn_listener(
    mut event_rx: broadcast::Receiver<ProcessEvent>,
    handle: Handle<AppTray>,
    notifier: Notifier,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    let ProcessEvent::StateChanged {
                        from,
                        to,
                        connection,
                    } = event
                    else {
                        continue;
                    };
                    apply_state_event(&handle, &notifier, from, to, connection).await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Broadcast keeps the newest messages; draining now applies
                    // the retained transitions and ends on the current state.
                    loop {
                        match event_rx.try_recv() {
                            Ok(ProcessEvent::StateChanged {
                                from,
                                to,
                                connection,
                            }) => {
                                apply_state_event(&handle, &notifier, from, to, connection).await;
                            }
                            Ok(_) => continue,
                            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                            Err(broadcast::error::TryRecvError::Empty) => break,
                            Err(broadcast::error::TryRecvError::Closed) => return,
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub struct TrayService;

impl TrayService {
    pub async fn spawn(
        event_rx: broadcast::Receiver<ProcessEvent>,
        notifier: Notifier,
        on_action: impl Fn(TrayAction) + Send + Sync + 'static,
        tray_id: String,
    ) -> Result<TrayHandle, ksni::Error> {
        let on_action = Arc::new(on_action);

        let icon_theme_dir = icons::setup_icon_theme();
        let icon_theme_path = icon_theme_dir
            .as_ref()
            .map(|d| d.path().join("hicolor").to_string_lossy().into_owned())
            .unwrap_or_default();

        let tray = AppTray {
            tray_id,
            process_state: ProcessState::Stopped,
            connection: None,
            on_action,
            _icon_theme_dir: icon_theme_dir,
            icon_theme_path,
        };

        let handle = tray.spawn().await?;
        let listener = spawn_listener(event_rx, handle.clone(), notifier.clone());

        Ok(TrayHandle {
            handle,
            listener,
            notifier,
        })
    }

    pub async fn spawn_with_data_dir(
        event_rx: broadcast::Receiver<ProcessEvent>,
        notifier: Notifier,
        on_action: impl Fn(TrayAction) + Send + Sync + 'static,
        data_dir: &Path,
        tray_id: String,
    ) -> Result<TrayHandle, ksni::Error> {
        let on_action = Arc::new(on_action);

        let icon_theme_dir = icons::setup_icon_theme();
        let icon_theme_path = icon_theme_dir
            .as_ref()
            .map(|d| d.path().join("hicolor").to_string_lossy().into_owned())
            .unwrap_or_default();

        let install_dir = data_dir.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || icons::install_icons(&install_dir)).await;

        let tray = AppTray {
            tray_id,
            process_state: ProcessState::Stopped,
            connection: None,
            on_action,
            _icon_theme_dir: icon_theme_dir,
            icon_theme_path,
        };

        let handle = tray.spawn().await?;
        let listener = spawn_listener(event_rx, handle.clone(), notifier.clone());

        Ok(TrayHandle {
            handle,
            listener,
            notifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;
    use v2ray_rs_core::models::{AutoResolveStrategy, BackendType, ConnectionNodeRef};

    use super::*;

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
            toggle_action(&ProcessState::Starting),
            (TrayAction::Disconnect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Running),
            (TrayAction::Disconnect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Stopped),
            (TrayAction::Connect, true)
        ));
        assert!(matches!(
            toggle_action(&ProcessState::Error("boom".into())),
            (TrayAction::Connect, true)
        ));
    }

    #[test]
    fn toggle_disabled_while_stopping() {
        let (_, enabled) = toggle_action(&ProcessState::Stopping);
        assert!(!enabled);
    }

    #[test]
    fn id_returns_injected_tray_id() {
        let tray = AppTray {
            tray_id: "com.github.v2ray-rs.dev".into(),
            process_state: ProcessState::Stopped,
            connection: None,
            on_action: Arc::new(|action: TrayAction| drop(action)),
            _icon_theme_dir: None,
            icon_theme_path: String::new(),
        };
        assert_eq!(Tray::id(&tray), "com.github.v2ray-rs.dev");
    }
}
