use std::path::PathBuf;

use adw::prelude::*;
use relm4::adw;
use relm4::prelude::*;

const DIALOG_WIDTH: i32 = 700;
const DIALOG_HEIGHT: i32 = 500;
const INVALID_NOTICE: &str = "The generated config is not valid JSON and cannot be previewed safely.\nEnable “Reveal raw” to see the original file content.";

pub struct ConfigPreviewDialog {
    path: PathBuf,
    absolute_path: String,
    text_buffer: gtk::TextBuffer,
    text_view: gtk::TextView,
    raw_content: String,
    redacted_content: Option<String>,
    reveal_raw: bool,
    status: Option<String>,
}

#[derive(Debug)]
pub enum ConfigPreviewInput {
    Refresh,
    SetReveal(bool),
    CopyPath,
    SetPath(PathBuf),
    Loaded(Result<String, std::io::Error>),
}

#[derive(Debug)]
pub enum ConfigPreviewCommandOutput {
    FileRead(Result<String, std::io::Error>),
}

#[relm4::component(pub)]
impl Component for ConfigPreviewDialog {
    type Init = PathBuf;
    type Input = ConfigPreviewInput;
    type Output = ();
    type CommandOutput = ConfigPreviewCommandOutput;

    view! {
        adw::Dialog {
            set_title: "Generated Config",
            set_content_width: DIALOG_WIDTH,
            set_content_height: DIALOG_HEIGHT,
            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
                set_margin_top: 12,
                set_margin_bottom: 12,
                set_margin_start: 12,
                set_margin_end: 12,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,

                    gtk::Button {
                        set_label: "Refresh",
                        connect_clicked[sender] => move |_| {
                            sender.input(ConfigPreviewInput::Refresh);
                        },
                    },

                    gtk::ToggleButton {
                        set_label: "Reveal raw",
                        #[watch]
                        set_active: model.reveal_raw,
                        connect_toggled[sender] => move |btn| {
                            sender.input(ConfigPreviewInput::SetReveal(btn.is_active()));
                        },
                    },

                    gtk::Button {
                        set_label: "Copy path",
                        connect_clicked[sender] => move |_| {
                            sender.input(ConfigPreviewInput::CopyPath);
                        },
                    },
                },

                gtk::ScrolledWindow {
                    set_vexpand: true,
                    #[watch]
                    set_visible: model.status.is_none(),

                    #[local_ref]
                    text_view -> gtk::TextView {
                        set_editable: false,
                        set_cursor_visible: false,
                        set_monospace: true,
                        set_wrap_mode: gtk::WrapMode::None,
                        set_left_margin: 12,
                        set_right_margin: 12,
                        set_top_margin: 12,
                        set_bottom_margin: 12,
                    },
                },

                adw::StatusPage {
                    set_icon_name: Some("document-open-symbolic"),
                    set_title: "Generated config not found",
                    #[watch]
                    set_description: model.status.as_deref(),
                    #[watch]
                    set_visible: model.status.is_some(),
                    set_vexpand: true,
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let path = init;
        let absolute_path = absolute_path_string(&path);
        let text_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let text_view = gtk::TextView::builder().buffer(&text_buffer).build();

        let model = ConfigPreviewDialog {
            path,
            absolute_path,
            text_buffer,
            text_view,
            raw_content: String::new(),
            redacted_content: None,
            reveal_raw: false,
            status: None,
        };

        let path_for_load = model.path.clone();
        sender.oneshot_command(async move {
            ConfigPreviewCommandOutput::FileRead(tokio::fs::read_to_string(&path_for_load).await)
        });

        let text_view = &model.text_view;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ConfigPreviewInput::Refresh => {
                let path = self.path.clone();
                sender.oneshot_command(async move {
                    ConfigPreviewCommandOutput::FileRead(tokio::fs::read_to_string(&path).await)
                });
            }
            ConfigPreviewInput::SetReveal(reveal) => {
                self.reveal_raw = reveal;
                self.update_text();
            }
            ConfigPreviewInput::CopyPath => {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&self.absolute_path);
                }
            }
            ConfigPreviewInput::SetPath(path) => {
                self.path = path;
                self.absolute_path = absolute_path_string(&self.path);
                sender.input(ConfigPreviewInput::Refresh);
            }
            ConfigPreviewInput::Loaded(result) => match result {
                Ok(raw) => {
                    self.status = None;
                    self.raw_content = raw;
                    self.redacted_content = v2ray_rs_core::config::redact_json(&self.raw_content);
                    self.reveal_raw = false;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.status = Some(format!(
                        "The expected config file has not been generated yet.\n{}",
                        self.absolute_path
                    ));
                    self.raw_content.clear();
                    self.redacted_content = None;
                    self.reveal_raw = false;
                }
                Err(e) => {
                    self.status = Some(format!("Failed to read generated config: {e}"));
                    self.raw_content.clear();
                    self.redacted_content = None;
                    self.reveal_raw = false;
                }
            },
        }
        if !matches!(msg, ConfigPreviewInput::SetReveal(_) | ConfigPreviewInput::CopyPath) {
            self.update_text();
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ConfigPreviewCommandOutput::FileRead(result) => {
                sender.input(ConfigPreviewInput::Loaded(result));
            }
        }
    }
}

impl ConfigPreviewDialog {
    fn update_text(&self) {
        let text = if let Some(status) = &self.status {
            status.clone()
        } else if self.reveal_raw {
            self.raw_content.clone()
        } else {
            self.redacted_content
                .clone()
                .unwrap_or_else(|| INVALID_NOTICE.to_string())
        };
        self.text_buffer.set_text(&text);
    }
}

fn absolute_path_string(path: &std::path::Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
