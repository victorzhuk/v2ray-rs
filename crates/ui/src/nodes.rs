use adw::prelude::*;
use relm4::adw;
use relm4::prelude::*;
use uuid::Uuid;

use v2ray_rs_core::models::{
    GrpcSettings, H2Settings, ManualNode, ProxyNode, ShadowsocksConfig, TlsSettings,
    TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings,
};

use crate::workspace::WorkspaceStore;

pub struct NodesPage {
    store: WorkspaceStore,
    nodes: Vec<ManualNode>,
    list_container: gtk::ListBox,
    load_error: Option<String>,
}

#[derive(Debug)]
pub enum NodesOutput {
    ActiveNodesChanged(bool),
    NodesChanged,
    Notice(String),
}

#[derive(Debug)]
pub enum NodesMsg {
    ToggleNode(Uuid),
    DeleteNode(Uuid),
    EditNode(Uuid),
    AddNode(ProxyNode),
    UpdateNode(Uuid, ProxyNode),
    ImportFromUrl(String),
    ResetStorage,
}

#[derive(Clone, Copy)]
enum ProtocolKind {
    Vless,
    Vmess,
    Shadowsocks,
    Trojan,
}

impl ProtocolKind {
    fn label(self) -> &'static str {
        match self {
            Self::Vless => "VLESS",
            Self::Vmess => "VMess",
            Self::Shadowsocks => "Shadowsocks",
            Self::Trojan => "Trojan",
        }
    }

    fn from_node(node: &ProxyNode) -> Self {
        match node {
            ProxyNode::Vless(_) => Self::Vless,
            ProxyNode::Vmess(_) => Self::Vmess,
            ProxyNode::Shadowsocks(_) => Self::Shadowsocks,
            ProxyNode::Trojan(_) => Self::Trojan,
        }
    }

    fn response_id(self) -> &'static str {
        match self {
            Self::Vless => "vless",
            Self::Vmess => "vmess",
            Self::Shadowsocks => "shadowsocks",
            Self::Trojan => "trojan",
        }
    }

    fn from_response_id(response: &str) -> Option<Self> {
        match response {
            "vless" => Some(Self::Vless),
            "vmess" => Some(Self::Vmess),
            "shadowsocks" => Some(Self::Shadowsocks),
            "trojan" => Some(Self::Trojan),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct CommonRows {
    address: adw::EntryRow,
    port: adw::SpinRow,
    remark: adw::EntryRow,
}

impl CommonRows {
    fn from_node(node: &ProxyNode) -> Self {
        Self::new(node.address(), node.port(), node.remark())
    }

    fn new(address: &str, port: u16, remark: Option<&str>) -> Self {
        Self {
            address: adw::EntryRow::builder()
                .title("Address")
                .text(address)
                .build(),
            port: spin_row("Port", port as f64, 1.0, 65535.0),
            remark: adw::EntryRow::builder()
                .title("Remark")
                .text(remark.unwrap_or_default())
                .build(),
        }
    }

    fn append_to(&self, group: &adw::PreferencesGroup) {
        group.add(&self.address);
        group.add(&self.port);
        group.add(&self.remark);
    }

    fn address(&self) -> String {
        trimmed_text(&self.address)
    }

    fn port(&self) -> u16 {
        self.port.value() as u16
    }

    fn remark(&self) -> Option<String> {
        trimmed_optional_text(&self.remark)
    }
}

#[derive(Clone)]
struct TransportRows {
    kind: adw::ComboRow,
    ws_path: adw::EntryRow,
    ws_host: adw::EntryRow,
    grpc_service_name: adw::EntryRow,
    grpc_multi_mode: adw::SwitchRow,
    h2_host: adw::EntryRow,
    h2_path: adw::EntryRow,
}

impl TransportRows {
    fn new(initial: &TransportSettings) -> Self {
        let selected = match initial {
            TransportSettings::Tcp => 0,
            TransportSettings::Ws(_) => 1,
            TransportSettings::Grpc(_) => 2,
            TransportSettings::H2(_) => 3,
        };
        let (ws_path, ws_host) = match initial {
            TransportSettings::Ws(ws) => (ws.path.clone(), ws.host.clone().unwrap_or_default()),
            _ => ("/".into(), String::new()),
        };
        let (grpc_service_name, grpc_multi_mode) = match initial {
            TransportSettings::Grpc(grpc) => (grpc.service_name.clone(), grpc.multi_mode),
            _ => (String::new(), false),
        };
        let (h2_host, h2_path) = match initial {
            TransportSettings::H2(h2) => (h2.host.join(","), h2.path.clone()),
            _ => (String::new(), "/".into()),
        };
        let rows = Self {
            kind: adw::ComboRow::builder()
                .title("Transport")
                .model(&gtk::StringList::new(&[
                    "TCP",
                    "WebSocket",
                    "gRPC",
                    "HTTP/2",
                ]))
                .selected(selected)
                .build(),
            ws_path: adw::EntryRow::builder()
                .title("WS Path")
                .text(&ws_path)
                .build(),
            ws_host: adw::EntryRow::builder()
                .title("WS Host")
                .text(&ws_host)
                .build(),
            grpc_service_name: adw::EntryRow::builder()
                .title("gRPC Service Name")
                .text(&grpc_service_name)
                .build(),
            grpc_multi_mode: adw::SwitchRow::builder()
                .title("gRPC Multi Mode")
                .active(grpc_multi_mode)
                .build(),
            h2_host: adw::EntryRow::builder()
                .title("HTTP/2 Hosts")
                .text(&h2_host)
                .build(),
            h2_path: adw::EntryRow::builder()
                .title("HTTP/2 Path")
                .text(&h2_path)
                .build(),
        };
        rows.sync_visibility();
        {
            let rows_clone = rows.clone();
            rows.kind.connect_selected_notify(move |_| {
                rows_clone.sync_visibility();
            });
        }
        rows
    }

    fn append_to(&self, group: &adw::PreferencesGroup) {
        group.add(&self.kind);
        group.add(&self.ws_path);
        group.add(&self.ws_host);
        group.add(&self.grpc_service_name);
        group.add(&self.grpc_multi_mode);
        group.add(&self.h2_host);
        group.add(&self.h2_path);
    }

    fn sync_visibility(&self) {
        let ws_selected = self.kind.selected() == 1;
        let grpc_selected = self.kind.selected() == 2;
        let h2_selected = self.kind.selected() == 3;
        self.ws_path.set_visible(ws_selected);
        self.ws_host.set_visible(ws_selected);
        self.grpc_service_name.set_visible(grpc_selected);
        self.grpc_multi_mode.set_visible(grpc_selected);
        self.h2_host.set_visible(h2_selected);
        self.h2_path.set_visible(h2_selected);
    }

    fn value(&self) -> TransportSettings {
        match self.kind.selected() {
            1 => TransportSettings::Ws(WsSettings {
                path: trimmed_text(&self.ws_path),
                host: trimmed_optional_text(&self.ws_host),
                headers: Default::default(),
            }),
            2 => TransportSettings::Grpc(GrpcSettings {
                service_name: trimmed_text(&self.grpc_service_name),
                multi_mode: self.grpc_multi_mode.is_active(),
            }),
            3 => TransportSettings::H2(H2Settings {
                host: comma_separated_values(&self.h2_host),
                path: trimmed_text(&self.h2_path),
            }),
            _ => TransportSettings::Tcp,
        }
    }
}

#[derive(Clone)]
struct TlsRows {
    enabled: adw::SwitchRow,
    server_name: adw::EntryRow,
    verify: adw::SwitchRow,
    alpn: adw::EntryRow,
    fingerprint: adw::EntryRow,
    reality: adw::SwitchRow,
    public_key: adw::EntryRow,
    short_id: adw::EntryRow,
    spider_x: adw::EntryRow,
}

impl TlsRows {
    fn new(initial: Option<&TlsSettings>) -> Self {
        let tls = initial.cloned().unwrap_or_default();
        let rows = Self {
            enabled: adw::SwitchRow::builder()
                .title("Enable TLS")
                .active(initial.is_some())
                .build(),
            server_name: adw::EntryRow::builder()
                .title("Server Name")
                .text(tls.server_name.as_deref().unwrap_or_default())
                .build(),
            verify: adw::SwitchRow::builder()
                .title("Verify Certificate")
                .active(tls.verify)
                .build(),
            alpn: adw::EntryRow::builder()
                .title("ALPN")
                .text(tls.alpn.join(","))
                .build(),
            fingerprint: adw::EntryRow::builder()
                .title("Fingerprint")
                .text(tls.fingerprint.as_deref().unwrap_or_default())
                .build(),
            reality: adw::SwitchRow::builder()
                .title("Reality")
                .active(tls.reality)
                .build(),
            public_key: adw::EntryRow::builder()
                .title("Reality Public Key")
                .text(tls.public_key.as_deref().unwrap_or_default())
                .build(),
            short_id: adw::EntryRow::builder()
                .title("Reality Short ID")
                .text(tls.short_id.as_deref().unwrap_or_default())
                .build(),
            spider_x: adw::EntryRow::builder()
                .title("Reality Spider X")
                .text(tls.spider_x.as_deref().unwrap_or_default())
                .build(),
        };
        rows.sync_visibility();
        {
            let rows_clone = rows.clone();
            rows.enabled.connect_active_notify(move |_| {
                rows_clone.sync_visibility();
            });
        }
        rows
    }

    fn append_to(&self, group: &adw::PreferencesGroup) {
        group.add(&self.enabled);
        group.add(&self.server_name);
        group.add(&self.verify);
        group.add(&self.alpn);
        group.add(&self.fingerprint);
        group.add(&self.reality);
        group.add(&self.public_key);
        group.add(&self.short_id);
        group.add(&self.spider_x);
    }

    fn sync_visibility(&self) {
        let enabled = self.enabled.is_active();
        self.server_name.set_visible(enabled);
        self.verify.set_visible(enabled);
        self.alpn.set_visible(enabled);
        self.fingerprint.set_visible(enabled);
        self.reality.set_visible(enabled);
        self.public_key.set_visible(enabled);
        self.short_id.set_visible(enabled);
        self.spider_x.set_visible(enabled);
    }

    fn value(&self) -> Option<TlsSettings> {
        if !self.enabled.is_active() {
            return None;
        }
        Some(TlsSettings {
            server_name: trimmed_optional_text(&self.server_name),
            alpn: comma_separated_values(&self.alpn),
            verify: self.verify.is_active(),
            fingerprint: trimmed_optional_text(&self.fingerprint),
            reality: self.reality.is_active(),
            public_key: trimmed_optional_text(&self.public_key),
            short_id: trimmed_optional_text(&self.short_id),
            spider_x: trimmed_optional_text(&self.spider_x),
        })
    }
}

enum NodeForm {
    Vless {
        common: CommonRows,
        uuid: adw::EntryRow,
        encryption: adw::EntryRow,
        flow: adw::EntryRow,
        transport: TransportRows,
        tls: TlsRows,
    },
    Vmess {
        common: CommonRows,
        uuid: adw::EntryRow,
        alter_id: adw::SpinRow,
        security: adw::EntryRow,
        transport: TransportRows,
        tls: TlsRows,
    },
    Shadowsocks {
        common: CommonRows,
        method: adw::EntryRow,
        password: adw::PasswordEntryRow,
    },
    Trojan {
        common: CommonRows,
        password: adw::PasswordEntryRow,
        transport: TransportRows,
        tls: TlsRows,
    },
}

impl NodeForm {
    fn from_node(node: &ProxyNode, content: &gtk::Box) -> Self {
        match node {
            ProxyNode::Vless(cfg) => {
                let common = CommonRows::from_node(node);
                let uuid = adw::EntryRow::builder()
                    .title("UUID")
                    .text(&cfg.uuid)
                    .build();
                let encryption = adw::EntryRow::builder()
                    .title("Encryption")
                    .text(cfg.encryption.as_deref().unwrap_or_default())
                    .build();
                let flow = adw::EntryRow::builder()
                    .title("Flow")
                    .text(cfg.flow.as_deref().unwrap_or_default())
                    .build();
                let transport = TransportRows::new(&cfg.transport);
                let tls = TlsRows::new(cfg.tls.as_ref());

                let connection_group = adw::PreferencesGroup::builder().title("Connection").build();
                common.append_to(&connection_group);
                connection_group.add(&uuid);
                connection_group.add(&encryption);
                connection_group.add(&flow);
                content.append(&connection_group);

                let transport_group = adw::PreferencesGroup::builder().title("Transport").build();
                transport.append_to(&transport_group);
                content.append(&transport_group);

                let tls_group = adw::PreferencesGroup::builder().title("TLS").build();
                tls.append_to(&tls_group);
                content.append(&tls_group);

                Self::Vless {
                    common,
                    uuid,
                    encryption,
                    flow,
                    transport,
                    tls,
                }
            }
            ProxyNode::Vmess(cfg) => {
                let common = CommonRows::from_node(node);
                let uuid = adw::EntryRow::builder()
                    .title("UUID")
                    .text(&cfg.uuid)
                    .build();
                let alter_id = spin_row("Alter ID", cfg.alter_id as f64, 0.0, 65535.0);
                let security = adw::EntryRow::builder()
                    .title("Security")
                    .text(&cfg.security)
                    .build();
                let transport = TransportRows::new(&cfg.transport);
                let tls = TlsRows::new(cfg.tls.as_ref());

                let connection_group = adw::PreferencesGroup::builder().title("Connection").build();
                common.append_to(&connection_group);
                connection_group.add(&uuid);
                connection_group.add(&alter_id);
                connection_group.add(&security);
                content.append(&connection_group);

                let transport_group = adw::PreferencesGroup::builder().title("Transport").build();
                transport.append_to(&transport_group);
                content.append(&transport_group);

                let tls_group = adw::PreferencesGroup::builder().title("TLS").build();
                tls.append_to(&tls_group);
                content.append(&tls_group);

                Self::Vmess {
                    common,
                    uuid,
                    alter_id,
                    security,
                    transport,
                    tls,
                }
            }
            ProxyNode::Shadowsocks(cfg) => {
                let common = CommonRows::from_node(node);
                let method = adw::EntryRow::builder()
                    .title("Method")
                    .text(&cfg.method)
                    .build();
                let password = adw::PasswordEntryRow::builder()
                    .title("Password")
                    .text(&cfg.password)
                    .build();

                let connection_group = adw::PreferencesGroup::builder().title("Connection").build();
                common.append_to(&connection_group);
                connection_group.add(&method);
                connection_group.add(&password);
                content.append(&connection_group);

                Self::Shadowsocks {
                    common,
                    method,
                    password,
                }
            }
            ProxyNode::Trojan(cfg) => {
                let common = CommonRows::from_node(node);
                let password = adw::PasswordEntryRow::builder()
                    .title("Password")
                    .text(&cfg.password)
                    .build();
                let transport = TransportRows::new(&cfg.transport);
                let tls = TlsRows::new(cfg.tls.as_ref());

                let connection_group = adw::PreferencesGroup::builder().title("Connection").build();
                common.append_to(&connection_group);
                connection_group.add(&password);
                content.append(&connection_group);

                let transport_group = adw::PreferencesGroup::builder().title("Transport").build();
                transport.append_to(&transport_group);
                content.append(&transport_group);

                let tls_group = adw::PreferencesGroup::builder().title("TLS").build();
                tls.append_to(&tls_group);
                content.append(&tls_group);

                Self::Trojan {
                    common,
                    password,
                    transport,
                    tls,
                }
            }
        }
    }

    fn to_node(&self) -> ProxyNode {
        match self {
            Self::Vless {
                common,
                uuid,
                encryption,
                flow,
                transport,
                tls,
            } => ProxyNode::Vless(VlessConfig {
                address: common.address(),
                port: common.port(),
                uuid: trimmed_text(uuid),
                encryption: trimmed_optional_text(encryption),
                flow: trimmed_optional_text(flow),
                transport: transport.value(),
                tls: tls.value(),
                remark: common.remark(),
            }),
            Self::Vmess {
                common,
                uuid,
                alter_id,
                security,
                transport,
                tls,
            } => ProxyNode::Vmess(VmessConfig {
                address: common.address(),
                port: common.port(),
                uuid: trimmed_text(uuid),
                alter_id: alter_id.value() as u32,
                security: trimmed_text(security),
                transport: transport.value(),
                tls: tls.value(),
                remark: common.remark(),
            }),
            Self::Shadowsocks {
                common,
                method,
                password,
            } => ProxyNode::Shadowsocks(ShadowsocksConfig {
                address: common.address(),
                port: common.port(),
                method: trimmed_text(method),
                password: trimmed_password_text(password),
                remark: common.remark(),
            }),
            Self::Trojan {
                common,
                password,
                transport,
                tls,
            } => ProxyNode::Trojan(TrojanConfig {
                address: common.address(),
                port: common.port(),
                password: trimmed_password_text(password),
                transport: transport.value(),
                tls: tls.value(),
                remark: common.remark(),
            }),
        }
    }
}

#[relm4::component(pub)]
impl Component for NodesPage {
    type Init = (WorkspaceStore, v2ray_rs_core::models::AppSettings);
    type Input = NodesMsg;
    type Output = NodesOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 0,

            gtk::Revealer {
                #[watch]
                set_reveal_child: model.load_error.is_some(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    set_margin_top: 6,
                    set_margin_bottom: 6,
                    set_margin_start: 12,
                    set_margin_end: 12,

                    gtk::Label {
                        set_xalign: 0.0,
                        set_hexpand: true,
                        add_css_class: "warning",
                        #[watch]
                        set_label: model
                            .load_error
                            .as_deref()
                            .unwrap_or("Failed to load manual nodes"),
                    },

                    gtk::Button {
                        set_label: "Reset Data",
                        add_css_class: "destructive-action",
                        connect_clicked => NodesMsg::ResetStorage,
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 6,
                set_margin_end: 6,

                gtk::Button {
                    set_icon_name: "document-open-recent-symbolic",
                    set_tooltip_text: Some("Import from URL"),
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.load_error.is_none(),
                    connect_clicked[sender] => move |_| {
                        show_import_dialog(sender.clone());
                    },
                },

                gtk::Button {
                    set_icon_name: "list-add-symbolic",
                    set_tooltip_text: Some("Add Manual Node"),
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.load_error.is_none(),
                    connect_clicked[sender] => move |_| {
                        show_protocol_picker(sender.clone());
                    },
                },
            },

            gtk::ScrolledWindow {
                set_vexpand: true,
                set_hexpand: true,

                #[wrap(Some)]
                set_child = &model.list_container.clone(),
            },
        }
    }

    fn init(
        init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (store, _settings) = init;
        let (nodes, load_error) = match store.load_manual_nodes() {
            Ok(nodes) => (nodes, None),
            Err(err) => (
                Vec::new(),
                Some(format!("Manual nodes are read-only: {err}")),
            ),
        };

        let list_container = gtk::ListBox::builder()
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["boxed-list"])
            .selection_mode(gtk::SelectionMode::None)
            .build();

        let model = NodesPage {
            store,
            nodes,
            list_container: list_container.clone(),
            load_error,
        };

        render_list(&model.nodes, &list_container, &sender);

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        let emit_active_nodes = |nodes: &[ManualNode], sender: &ComponentSender<Self>| {
            let has_active = nodes.iter().any(|n| n.enabled);
            let _ = sender.output(NodesOutput::ActiveNodesChanged(has_active));
        };

        let mut changed = false;

        if self.load_error.is_some() && !matches!(msg, NodesMsg::ResetStorage) {
            return;
        }

        match msg {
            NodesMsg::ToggleNode(id) => {
                if let Some(pos) = self.nodes.iter().position(|n| n.id == id) {
                    self.nodes[pos].enabled = !self.nodes[pos].enabled;
                    if persist_manual_nodes(&self.store, &self.nodes) {
                        changed = true;
                    } else {
                        self.nodes[pos].enabled = !self.nodes[pos].enabled;
                    }
                }
            }
            NodesMsg::DeleteNode(id) => {
                if let Some(pos) = self.nodes.iter().position(|n| n.id == id) {
                    let removed = self.nodes.remove(pos);
                    if persist_manual_nodes(&self.store, &self.nodes) {
                        changed = true;
                    } else {
                        self.nodes.insert(pos, removed);
                    }
                }
            }
            NodesMsg::EditNode(id) => {
                let node = match self.nodes.iter().find(|n| n.id == id) {
                    Some(n) => n.node.clone(),
                    None => return,
                };
                show_edit_dialog(id, node, sender.clone());
            }
            NodesMsg::AddNode(node) => {
                if let Err(err) = node.validate() {
                    let _ = sender.output(NodesOutput::Notice(err.to_string()));
                    return;
                }
                let manual = ManualNode::new(node);
                self.nodes.push(manual);
                if persist_manual_nodes(&self.store, &self.nodes) {
                    changed = true;
                } else {
                    let _ = self.nodes.pop();
                }
            }
            NodesMsg::UpdateNode(id, node) => {
                if let Err(err) = node.validate() {
                    let _ = sender.output(NodesOutput::Notice(err.to_string()));
                    return;
                }
                if let Some(pos) = self.nodes.iter().position(|n| n.id == id) {
                    let previous = self.nodes[pos].node.clone();
                    self.nodes[pos].node = node;
                    if persist_manual_nodes(&self.store, &self.nodes) {
                        changed = true;
                    } else {
                        self.nodes[pos].node = previous;
                    }
                }
            }
            NodesMsg::ImportFromUrl(uri) => {
                match v2ray_rs_subscription::parse_uri(&uri) {
                    Ok(node) => {
                        if let Err(err) = node.validate() {
                            let _ = sender.output(NodesOutput::Notice(format!("Invalid node: {err}")));
                            return;
                        }
                        let manual = ManualNode::new(node);
                        self.nodes.push(manual);
                        if persist_manual_nodes(&self.store, &self.nodes) {
                            changed = true;
                        } else {
                            let _ = self.nodes.pop();
                        }
                    }
                    Err(err) => {
                        let _ = sender.output(NodesOutput::Notice(format!("Failed to parse URI: {err}")));
                    }
                }
            }
            NodesMsg::ResetStorage => match self.store.reset_manual_nodes() {
                Ok(()) => {
                    self.nodes.clear();
                    self.load_error = None;
                }
                Err(err) => {
                    log::error!("reset manual nodes: {err}");
                }
            },
        }

        emit_active_nodes(&self.nodes, &sender);
        if changed {
            let _ = sender.output(NodesOutput::NodesChanged);
        }
        render_list(&self.nodes, &self.list_container, &sender);
    }
}

fn persist_manual_nodes(store: &WorkspaceStore, nodes: &[ManualNode]) -> bool {
    match store.save_manual_nodes(nodes) {
        Ok(()) => true,
        Err(e) => {
            log::error!("save manual nodes: {e}");
            false
        }
    }
}

fn render_list(
    nodes: &[ManualNode],
    container: &gtk::ListBox,
    sender: &ComponentSender<NodesPage>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if nodes.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("network-server-symbolic")
            .title("No Manual Nodes")
            .description("Add a manual node to get started")
            .build();
        let row = gtk::ListBoxRow::builder()
            .selectable(false)
            .activatable(false)
            .child(&empty)
            .build();
        container.append(&row);
        return;
    }

    for node in nodes {
        let node_row = build_node_row(node, sender);
        container.append(&node_row);
    }
}

fn build_node_row(node: &ManualNode, sender: &ComponentSender<NodesPage>) -> adw::ActionRow {
    let protocol = match &node.node {
        ProxyNode::Vless(_) => "VLESS",
        ProxyNode::Vmess(_) => "VMESS",
        ProxyNode::Shadowsocks(_) => "SS",
        ProxyNode::Trojan(_) => "TROJAN",
    };

    let address = format!("{}:{}", node.node.address(), node.node.port());
    let name = node.node.remark().unwrap_or("Unnamed Node");

    let row = adw::ActionRow::builder()
        .title(name)
        .subtitle(&address)
        .build();

    if !node.enabled {
        row.set_opacity(0.5);
    }

    let badge = gtk::Label::builder()
        .label(protocol)
        .css_classes(["caption", "accent"])
        .valign(gtk::Align::Center)
        .build();
    row.add_prefix(&badge);

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .build();
    menu_btn.add_css_class("flat");

    let popover = gtk::Popover::new();
    let popover_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    let edit_btn = gtk::Button::builder()
        .label("Edit")
        .has_frame(false)
        .build();
    {
        let id = node.id;
        let s = sender.clone();
        let p = popover.clone();
        edit_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(NodesMsg::EditNode(id));
        });
    }

    let delete_btn = gtk::Button::builder()
        .label("Delete")
        .has_frame(false)
        .build();
    delete_btn.add_css_class("destructive-action");
    {
        let id = node.id;
        let s = sender.clone();
        let p = popover.clone();
        delete_btn.connect_clicked(move |_| {
            p.popdown();
            show_delete_dialog(id, s.clone());
        });
    }

    popover_box.append(&edit_btn);
    popover_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_box.append(&delete_btn);
    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    row.add_suffix(&menu_btn);

    let node_toggle = gtk::Switch::builder()
        .active(node.enabled)
        .valign(gtk::Align::Center)
        .build();
    {
        let id = node.id;
        let s = sender.clone();
        node_toggle.connect_active_notify(move |_| {
            s.input(NodesMsg::ToggleNode(id));
        });
    }
    row.add_suffix(&node_toggle);

    row
}

fn show_protocol_picker(sender: ComponentSender<NodesPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Add Manual Node")
        .body("Choose the protocol for the new node")
        .build();

    dialog.add_response("cancel", "Cancel");
    for protocol in [
        ProtocolKind::Vless,
        ProtocolKind::Vmess,
        ProtocolKind::Shadowsocks,
        ProtocolKind::Trojan,
    ] {
        dialog.add_response(protocol.response_id(), protocol.label());
    }
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if let Some(protocol) = ProtocolKind::from_response_id(response) {
            show_node_form(None, default_node_for_protocol(protocol), sender.clone());
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_edit_dialog(id: Uuid, node: ProxyNode, sender: ComponentSender<NodesPage>) {
    show_node_form(Some(id), node, sender);
}

fn show_node_form(edit_id: Option<Uuid>, node: ProxyNode, sender: ComponentSender<NodesPage>) {
    let protocol = ProtocolKind::from_node(&node);
    let heading = if edit_id.is_some() {
        format!("Edit {} Node", protocol.label())
    } else {
        format!("Add {} Node", protocol.label())
    };

    let dialog = adw::AlertDialog::builder().heading(&heading).build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", if edit_id.is_some() { "Save" } else { "Add" });
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let form = NodeForm::from_node(&node, &content);
    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let node = form.to_node();
        if let Some(id) = edit_id {
            sender.input(NodesMsg::UpdateNode(id, node));
        } else {
            sender.input(NodesMsg::AddNode(node));
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_delete_dialog(id: Uuid, sender: ComponentSender<NodesPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete Manual Node")
        .body("Are you sure you want to delete this manual node?")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(NodesMsg::DeleteNode(id));
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_import_dialog(sender: ComponentSender<NodesPage>) {
    let entry = gtk::Entry::builder()
        .placeholder_text("vless://uuid@example.com:443#remark")
        .build();

    let dialog = adw::AlertDialog::builder()
        .heading("Import from URL")
        .body("Paste a proxy URI. Supported formats: vless://, vmess://, ss://, trojan://")
        .extra_child(&entry)
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("import", "Import");
    dialog.set_response_appearance("import", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("import"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |dlg, response| {
        if response == "import" {
            let uri = entry.text().to_string();
            if !uri.is_empty() {
                sender.input(NodesMsg::ImportFromUrl(uri));
            }
        }
        dlg.close();
    });

    dialog.present(crate::active_window().as_ref());
}

fn default_node_for_protocol(protocol: ProtocolKind) -> ProxyNode {
    match protocol {
        ProtocolKind::Vless => ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: Uuid::new_v4().to_string(),
            encryption: Some("none".into()),
            flow: None,
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                server_name: Some("example.com".into()),
                ..Default::default()
            }),
            remark: Some("New VLESS Node".into()),
        }),
        ProtocolKind::Vmess => ProxyNode::Vmess(VmessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: Uuid::new_v4().to_string(),
            alter_id: 0,
            security: "auto".into(),
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("New VMess Node".into()),
        }),
        ProtocolKind::Shadowsocks => ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "password".into(),
            remark: Some("New Shadowsocks Node".into()),
        }),
        ProtocolKind::Trojan => ProxyNode::Trojan(TrojanConfig {
            address: "example.com".into(),
            port: 443,
            password: "password".into(),
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                server_name: Some("example.com".into()),
                ..Default::default()
            }),
            remark: Some("New Trojan Node".into()),
        }),
    }
}

fn spin_row(title: &str, value: f64, min: f64, max: f64) -> adw::SpinRow {
    adw::SpinRow::builder()
        .title(title)
        .adjustment(&gtk::Adjustment::new(value, min, max, 1.0, 0.0, 0.0))
        .build()
}

fn trimmed_text(row: &adw::EntryRow) -> String {
    row.text().trim().to_string()
}

fn trimmed_password_text(row: &adw::PasswordEntryRow) -> String {
    row.text().trim().to_string()
}

fn trimmed_optional_text(row: &adw::EntryRow) -> Option<String> {
    let value = trimmed_text(row);
    (!value.is_empty()).then_some(value)
}

fn comma_separated_values(row: &adw::EntryRow) -> Vec<String> {
    row.text()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
