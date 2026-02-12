use adw::prelude::*;
use relm4::adw;
use relm4::prelude::*;

pub struct LogsPage {
    running: bool,
    log_buffer: gtk::TextBuffer,
    text_view: gtk::TextView,
}

#[derive(Debug)]
pub enum LogsMsg {
    AppendLine(String),
    Clear,
    SetRunning(bool),
}

#[relm4::component(pub)]
impl SimpleComponent for LogsPage {
    type Init = ();
    type Input = LogsMsg;
    type Output = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 0,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 6,
                set_margin_end: 6,

                gtk::Button {
                    set_icon_name: "edit-clear-all-symbolic",
                    set_tooltip_text: Some("Clear logs"),
                    add_css_class: "flat",
                    connect_clicked => LogsMsg::Clear,
                },
            },

            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::Crossfade,
                set_transition_duration: 200,
                #[watch]
                set_visible_child_name: if model.running { "logs" } else { "empty" },

                add_named[Some("empty")] = &adw::StatusPage {
                    set_title: "Process Not Running",
                    set_description: Some("Start the proxy to see logs"),
                    set_icon_name: Some("network-vpn-disconnected-symbolic"),
                },

                add_named[Some("logs")] = &gtk::ScrolledWindow {
                    set_vexpand: true,

                    #[local_ref]
                    text_view -> gtk::TextView {
                        set_editable: false,
                        set_cursor_visible: false,
                        set_monospace: true,
                        set_left_margin: 12,
                        set_right_margin: 12,
                        set_top_margin: 12,
                        set_bottom_margin: 12,
                        set_wrap_mode: gtk::WrapMode::Word,
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let log_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let text_view = gtk::TextView::builder().buffer(&log_buffer).build();

        let model = LogsPage {
            running: false,
            log_buffer: log_buffer.clone(),
            text_view: text_view.clone(),
        };

        let text_view = &model.text_view;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            LogsMsg::AppendLine(line) => {
                let mut end_iter = self.log_buffer.end_iter();
                if self.log_buffer.char_count() > 0 {
                    self.log_buffer.insert(&mut end_iter, "\n");
                    end_iter = self.log_buffer.end_iter();
                }
                self.log_buffer.insert(&mut end_iter, &line);

                if let Some(mark) = self.log_buffer.mark("insert") {
                    let end = self.log_buffer.end_iter();
                    self.log_buffer.move_mark(&mark, &end);
                    self.text_view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0);
                }
            }
            LogsMsg::Clear => {
                let mut start = self.log_buffer.start_iter();
                let mut end = self.log_buffer.end_iter();
                self.log_buffer.delete(&mut start, &mut end);
            }
            LogsMsg::SetRunning(running) => {
                self.running = running;
            }
        }
    }
}
