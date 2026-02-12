use std::sync::mpsc;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Handle, Tray, TrayMethods};
use tokio::sync::broadcast;
use v2ray_rs_process::{ProcessEvent, ProcessState};

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
    action_rx: mpsc::Receiver<TrayAction>,
}

impl TrayHandle {
    pub fn try_recv_action(&self) -> Option<TrayAction> {
        self.action_rx.try_recv().ok()
    }

    pub async fn update_state(&self, state: ProcessState) {
        self.handle
            .update(move |tray| {
                tray.process_state = state;
            })
            .await;
    }

    pub async fn shutdown(&self) {
        self.handle.shutdown().await;
    }
}

struct AppTray {
    process_state: ProcessState,
    action_tx: mpsc::Sender<TrayAction>,
}

impl Tray for AppTray {
    fn id(&self) -> String {
        "v2ray-rs".into()
    }

    fn title(&self) -> String {
        "V2Ray Manager".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        match &self.process_state {
            ProcessState::Running => icons::connected_pixmap(),
            ProcessState::Error(_) => icons::error_pixmap(),
            _ => icons::disconnected_pixmap(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let connected = self.process_state == ProcessState::Running;

        let toggle = if connected {
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Disconnect".into(),
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::Disconnect);
                }),
                ..Default::default()
            }
        } else {
            let starting = matches!(
                self.process_state,
                ProcessState::Starting | ProcessState::Stopping
            );
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Connect".into(),
                enabled: !starting,
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::Connect);
                }),
                ..Default::default()
            }
        };

        let status_label = match &self.process_state {
            ProcessState::Stopped => "Status: Disconnected",
            ProcessState::Starting => "Status: Connecting...",
            ProcessState::Running => "Status: Connected",
            ProcessState::Stopping => "Status: Disconnecting...",
            ProcessState::Error(msg) => return self.menu_with_error(toggle, msg),
        };

        let show_window = {
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Open Main Window".into(),
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::ShowWindow);
                }),
                ..Default::default()
            }
        };

        let quit = {
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::Quit);
                }),
                ..Default::default()
            }
        };

        vec![
            toggle.into(),
            MenuItem::Separator,
            StandardItem {
                label: status_label.into(),
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

impl AppTray {
    fn menu_with_error(&self, toggle: StandardItem<Self>, msg: &str) -> Vec<MenuItem<Self>> {
        let show_window = {
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Open Main Window".into(),
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::ShowWindow);
                }),
                ..Default::default()
            }
        };

        let quit = {
            let tx = self.action_tx.clone();
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    let _ = tx.send(TrayAction::Quit);
                }),
                ..Default::default()
            }
        };

        let truncated = if msg.len() > 50 {
            let boundary = msg.char_indices()
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

pub struct TrayService;

impl TrayService {
    pub async fn spawn(
        mut event_rx: broadcast::Receiver<ProcessEvent>,
        notifier: Notifier,
    ) -> Result<TrayHandle, ksni::Error> {
        let (action_tx, action_rx) = mpsc::channel();

        let tray = AppTray {
            process_state: ProcessState::Stopped,
            action_tx,
        };

        let handle = tray.spawn().await?;
        let update_handle = handle.clone();

        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        if let ProcessEvent::StateChanged { from, to } = event {
                            let state = to.clone();
                            update_handle
                                .update(move |tray| {
                                    tray.process_state = state;
                                })
                                .await;
                            let n = notifier.clone();
                            tokio::task::spawn_blocking(move || {
                                n.on_state_change(&from, &to);
                            });
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(TrayHandle { handle, action_rx })
    }
}
