use relm4::prelude::*;
use relm4::adw;
use adw::prelude::*;
use v2ray_rs_process::ProcessState;

pub struct StatusBar {
    connected: bool,
    status_text: String,
    button_enabled: bool,
}

#[derive(Debug)]
pub enum StatusBarMsg {
    SetConnected(bool),
    SetState(ProcessState),
    SetStatusText(String),
    ToggleConnection,
}

#[derive(Debug)]
pub enum StatusBarOutput {
    Connect,
    Disconnect,
}

#[relm4::component(pub)]
impl SimpleComponent for StatusBar {
    type Init = ();
    type Input = StatusBarMsg;
    type Output = StatusBarOutput;

    view! {
        gtk::ActionBar {
            pack_start = &gtk::Label {
                set_hexpand: true,
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: &model.status_text,
            },

            pack_end = &gtk::Button {
                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(if model.connected { "network-wireless-disabled-symbolic" } else { "network-wireless-symbolic" }),
                    },
                    gtk::Label {
                        #[watch]
                        set_label: if model.connected { "Disconnect" } else { "Connect" },
                    },
                },
                #[watch]
                set_sensitive: model.button_enabled,
                #[watch]
                set_css_classes: &[if model.connected { "destructive-action" } else { "suggested-action" }],
                connect_clicked => StatusBarMsg::ToggleConnection,
            },
        }
    }

    fn init(_init: Self::Init, root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = StatusBar {
            connected: false,
            status_text: "Disconnected".into(),
            button_enabled: true,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            StatusBarMsg::SetConnected(connected) => {
                self.connected = connected;
                self.status_text = if connected {
                    "Connected".into()
                } else {
                    "Disconnected".into()
                };
            }
            StatusBarMsg::SetState(state) => {
                match state {
                    ProcessState::Stopped => {
                        self.connected = false;
                        self.status_text = "Disconnected".into();
                        self.button_enabled = true;
                    }
                    ProcessState::Starting => {
                        self.connected = false;
                        self.status_text = "Connecting...".into();
                        self.button_enabled = false;
                    }
                    ProcessState::Running => {
                        self.connected = true;
                        self.status_text = "Connected".into();
                        self.button_enabled = true;
                    }
                    ProcessState::Stopping => {
                        self.connected = true;
                        self.status_text = "Disconnecting...".into();
                        self.button_enabled = false;
                    }
                    ProcessState::Error(ref msg) => {
                        self.connected = false;
                        self.status_text = format!("Error: {}", msg);
                        self.button_enabled = true;
                    }
                }
            }
            StatusBarMsg::SetStatusText(text) => {
                self.status_text = text;
            }
            StatusBarMsg::ToggleConnection => {
                if self.connected {
                    let _ = sender.output(StatusBarOutput::Disconnect);
                } else {
                    let _ = sender.output(StatusBarOutput::Connect);
                }
            }
        }
    }
}
