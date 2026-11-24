use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use adw::prelude::*;
use relm4::adw;
use relm4::prelude::*;
use uuid::Uuid;

use v2ray_rs_core::models::{
    GrpcSettings, H2Settings, ManualNode, ProxyNode, ShadowsocksConfig, TlsSettings,
    TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings, XhttpSettings,
};

use crate::i18n::tr;
use crate::workspace::{PersistenceReceipt, WorkspaceStore};

pub struct NodesPage {
    store: WorkspaceStore,
    /// Ordered edits: the committed list plus the submitted tail.
    draft: NodeDraft,
    /// Displayed list: the committed snapshot plus every pending edit.
    nodes: Vec<ManualNode>,
    /// Whether a failed write in the current batch was already reported.
    failure_reported: bool,
    list_container: gtk::ListBox,
    load_error: Option<String>,
    active_node: Option<Uuid>,
}

/// One ordered write submitted to the workspace writer for the node list.
#[derive(Clone, Debug)]
enum NodeOp {
    Toggle(Uuid),
    Delete(Uuid),
    Add(ManualNode),
    Update(Uuid, ProxyNode),
    Reset,
}

impl NodeOp {
    /// Applies the write to the latest committed list. A missing target is a
    /// no-op: another writer may already have removed the node.
    fn apply(self, nodes: &mut Vec<ManualNode>) {
        match self {
            Self::Toggle(id) => {
                if let Some(node) = nodes.iter_mut().find(|node| node.id == id) {
                    node.enabled = !node.enabled;
                }
            }
            Self::Delete(id) => nodes.retain(|node| node.id != id),
            Self::Add(node) => nodes.push(node),
            Self::Update(id, proxy) => {
                if let Some(node) = nodes.iter_mut().find(|node| node.id == id) {
                    node.node = proxy;
                }
            }
            Self::Reset => nodes.clear(),
        }
    }
}

/// The receipt's outer error means the writer stopped without answering.
const WRITER_STOPPED: &str = "workspace writer stopped";

/// Ordered node edits for one collection.
///
/// The displayed list is the last acknowledged list plus every edit still in
/// the writer queue. Each accepted edit is enqueued synchronously in
/// submission order, so the writer FIFO - and any later barrier - covers it.
/// Receipts settle in the same order; a failed receipt rolls back only its
/// own edit, because later edits were already enqueued against the same
/// committed base.
#[derive(Default)]
struct NodeDraft {
    committed: Vec<ManualNode>,
    submitted: VecDeque<NodeOp>,
}

impl NodeDraft {
    fn seed(&mut self, committed: Vec<ManualNode>) {
        self.committed = committed;
    }

    fn stage(&mut self, op: NodeOp) {
        self.submitted.push_back(op);
    }

    /// Test-only: exposing the pending head lets the draft tests prove that
    /// receipts settle strictly in submission order.
    #[cfg(test)]
    fn front(&self) -> Option<NodeOp> {
        self.submitted.front().cloned()
    }

    /// Retires the acknowledged write and adopts the list the writer returned.
    fn complete(&mut self, committed: Vec<ManualNode>) {
        self.committed = committed;
        self.submitted.pop_front();
    }

    /// Drops the rejected write. Later submissions stay queued: they were
    /// enqueued before this receipt and the writer will still settle them.
    fn fail(&mut self) {
        self.submitted.pop_front();
    }

    fn is_empty(&self) -> bool {
        self.submitted.is_empty()
    }

    fn committed_active(&self) -> bool {
        self.committed.iter().any(|node| node.enabled)
    }

    fn display(&self) -> Vec<ManualNode> {
        let mut nodes = self.committed.clone();
        for op in &self.submitted {
            op.clone().apply(&mut nodes);
        }
        nodes
    }
}

/// The receipt for either kind of node write, normalized to its committed list.
enum NodeWrite {
    Edit(PersistenceReceipt<((), Arc<Vec<ManualNode>>)>),
    Reset(PersistenceReceipt<Arc<Vec<ManualNode>>>),
}

#[derive(Debug)]
pub enum NodesOutput {
    ActiveNodesChanged(bool),
    NodesChanged,
    ConnectNode(Uuid),
    Notice(String),
}

#[derive(Debug)]
pub enum NodesMsg {
    ToggleNode(Uuid),
    DeleteNode(Uuid),
    EditNode(Uuid),
    ConnectNode(Uuid),
    AddNode(ProxyNode),
    UpdateNode(Uuid, ProxyNode),
    ImportFromUrl(String),
    ResetStorage,
    Reload,
    SetActiveNode(Option<Uuid>),
    /// Internal: an ordered node write settled with its durable list or the
    /// error that kept the previous snapshot.
    SaveFinished(Result<Arc<Vec<ManualNode>>, String>),
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
    ws_headers: HashMap<String, String>,
    grpc_service_name: adw::EntryRow,
    grpc_multi_mode: adw::SwitchRow,
    h2_host: adw::EntryRow,
    h2_path: adw::EntryRow,
    xhttp_path: adw::EntryRow,
    xhttp_host: adw::EntryRow,
    xhttp_mode: adw::EntryRow,
}

impl TransportRows {
    fn new(initial: &TransportSettings) -> Self {
        let selected = match initial {
            TransportSettings::Tcp => 0,
            TransportSettings::Ws(_) => 1,
            TransportSettings::Grpc(_) => 2,
            TransportSettings::H2(_) => 3,
            TransportSettings::Xhttp(_) => 4,
        };
        let (ws_path, ws_host, ws_headers) = match initial {
            TransportSettings::Ws(ws) => (
                ws.path.clone(),
                ws.host.clone().unwrap_or_default(),
                ws.headers.clone(),
            ),
            _ => ("/".into(), String::new(), HashMap::new()),
        };
        let (grpc_service_name, grpc_multi_mode) = match initial {
            TransportSettings::Grpc(grpc) => (grpc.service_name.clone(), grpc.multi_mode),
            _ => (String::new(), false),
        };
        let (h2_host, h2_path) = match initial {
            TransportSettings::H2(h2) => (h2.host.join(","), h2.path.clone()),
            _ => (String::new(), "/".into()),
        };
        let (xhttp_path, xhttp_host, xhttp_mode) = match initial {
            TransportSettings::Xhttp(xhttp) => (
                xhttp.path.clone(),
                xhttp.host.clone().unwrap_or_default(),
                xhttp.mode.clone(),
            ),
            _ => ("/".into(), String::new(), "auto".into()),
        };
        let rows = Self {
            kind: adw::ComboRow::builder()
                .title("Transport")
                .model(&gtk::StringList::new(&[
                    "TCP",
                    "WebSocket",
                    "gRPC",
                    "HTTP/2",
                    "XHTTP",
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
            ws_headers,
            grpc_service_name: adw::EntryRow::builder()
                .title("gRPC Service Name")
                .text(&grpc_service_name)
                .build(),
            grpc_multi_mode: adw::SwitchRow::builder()
                .title("gRPC Multi Mode")
                .subtitle("Not used by sing-box")
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
            xhttp_path: adw::EntryRow::builder()
                .title("XHTTP Path")
                .text(&xhttp_path)
                .build(),
            xhttp_host: adw::EntryRow::builder()
                .title("XHTTP Host")
                .text(&xhttp_host)
                .build(),
            xhttp_mode: adw::EntryRow::builder()
                .title("XHTTP Mode")
                .text(&xhttp_mode)
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
        group.add(&self.xhttp_path);
        group.add(&self.xhttp_host);
        group.add(&self.xhttp_mode);
    }

    fn sync_visibility(&self) {
        let ws_selected = self.kind.selected() == 1;
        let grpc_selected = self.kind.selected() == 2;
        let h2_selected = self.kind.selected() == 3;
        let xhttp_selected = self.kind.selected() == 4;
        self.ws_path.set_visible(ws_selected);
        self.ws_host.set_visible(ws_selected);
        self.grpc_service_name.set_visible(grpc_selected);
        self.grpc_multi_mode.set_visible(grpc_selected);
        self.h2_host.set_visible(h2_selected);
        self.h2_path.set_visible(h2_selected);
        self.xhttp_path.set_visible(xhttp_selected);
        self.xhttp_host.set_visible(xhttp_selected);
        self.xhttp_mode.set_visible(xhttp_selected);
    }

    fn value(&self) -> TransportSettings {
        match self.kind.selected() {
            1 => TransportSettings::Ws(WsSettings {
                path: trimmed_text(&self.ws_path),
                host: trimmed_optional_text(&self.ws_host),
                headers: self.ws_headers.clone(),
            }),
            2 => TransportSettings::Grpc(GrpcSettings {
                service_name: trimmed_text(&self.grpc_service_name),
                multi_mode: self.grpc_multi_mode.is_active(),
            }),
            3 => TransportSettings::H2(H2Settings {
                host: comma_separated_values(&self.h2_host),
                path: trimmed_text(&self.h2_path),
            }),
            4 => {
                let mode = trimmed_text(&self.xhttp_mode);
                TransportSettings::Xhttp(XhttpSettings {
                    path: trimmed_text(&self.xhttp_path),
                    host: trimmed_optional_text(&self.xhttp_host),
                    mode: if mode.is_empty() { "auto".into() } else { mode },
                })
            }
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
                .title("Reality Spider X (not used by sing-box)")
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

#[derive(Clone)]
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

    fn validation_error(&self) -> Option<&'static str> {
        let (common, credential) = match self {
            Self::Vless { common, uuid, .. } => (common, Some(uuid.text().to_string())),
            Self::Vmess { common, uuid, .. } => (common, Some(uuid.text().to_string())),
            Self::Shadowsocks {
                common,
                method,
                password,
            } => {
                if method.text().trim().is_empty() {
                    return Some("A cipher method is required");
                }
                (common, Some(password.text().to_string()))
            }
            Self::Trojan {
                common, password, ..
            } => (common, Some(password.text().to_string())),
        };

        if common.address().is_empty() {
            return Some("An address is required");
        }
        if credential.is_some_and(|text| text.trim().is_empty()) {
            return Some("Credentials are required");
        }
        None
    }

    fn validation_entries(&self) -> Vec<adw::EntryRow> {
        match self {
            Self::Vless { common, uuid, .. } | Self::Vmess { common, uuid, .. } => {
                vec![common.address.clone(), uuid.clone()]
            }
            Self::Shadowsocks {
                common,
                method,
                password,
            } => vec![
                common.address.clone(),
                method.clone(),
                password.clone().upcast(),
            ],
            Self::Trojan {
                common, password, ..
            } => vec![common.address.clone(), password.clone().upcast()],
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
                            .unwrap_or(&tr("Failed to load manual nodes")),
                    },

                    gtk::Button {
                        set_label: &tr("Reset Data"),
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
                    set_icon_name: "edit-paste-symbolic",
                    set_tooltip_text: Some(&tr("Import from URL")),
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.load_error.is_none(),
                    connect_clicked[sender] => move |_| {
                        show_import_dialog(sender.clone());
                    },
                },

                gtk::Button {
                    set_icon_name: "list-add-symbolic",
                    set_tooltip_text: Some(&tr("Add Manual Node")),
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
        // Warm bootstrap publishes the node list before the interactive app
        // starts; this reads the shared snapshot instead of loading from disk
        // on the GTK thread.
        let (committed, load_error) = match store.manual_nodes_snapshot() {
            Ok(nodes) => ((*nodes).clone(), None),
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

        let mut draft = NodeDraft::default();
        draft.seed(committed.clone());
        let model = NodesPage {
            store,
            draft,
            nodes: committed,
            failure_reported: false,
            list_container: list_container.clone(),
            load_error,
            active_node: None,
        };

        render_list(&model.nodes, &list_container, &sender, None);

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        let mut committed_changed = false;
        let mut report = None;

        if self.load_error.is_some()
            && !matches!(
                &msg,
                NodesMsg::ResetStorage | NodesMsg::Reload | NodesMsg::SaveFinished(_)
            )
        {
            return;
        }

        match msg {
            NodesMsg::Reload => match self.store.manual_nodes_snapshot() {
                Ok(nodes) => {
                    // A pending queue owns the base: its receipts publish the
                    // durable list, so a reload never reseeds beneath it.
                    if self.draft.is_empty() {
                        self.draft.seed((*nodes).clone());
                        self.nodes = self.draft.display();
                    }
                    self.load_error = None;
                }
                Err(err) => {
                    self.load_error = Some(format!("Manual nodes are read-only: {err}"));
                }
            },
            NodesMsg::ToggleNode(id) => {
                if self.nodes.iter().any(|node| node.id == id) {
                    self.stage(NodeOp::Toggle(id), &sender);
                }
            }
            NodesMsg::DeleteNode(id) => {
                if self.nodes.iter().any(|node| node.id == id) {
                    self.stage(NodeOp::Delete(id), &sender);
                }
            }
            NodesMsg::EditNode(id) => {
                let node = match self.nodes.iter().find(|node| node.id == id) {
                    Some(node) => node.node.clone(),
                    None => return,
                };
                show_edit_dialog(id, node, sender.clone());
            }
            NodesMsg::ConnectNode(id) => {
                let _ = sender.output(NodesOutput::ConnectNode(id));
            }
            NodesMsg::AddNode(node) => {
                if let Err(err) = node.validate() {
                    let _ = sender.output(NodesOutput::Notice(err.to_string()));
                    return;
                }
                self.stage(NodeOp::Add(ManualNode::new(node)), &sender);
            }
            NodesMsg::UpdateNode(id, node) => {
                if let Err(err) = node.validate() {
                    let _ = sender.output(NodesOutput::Notice(err.to_string()));
                    return;
                }
                if self.nodes.iter().any(|existing| existing.id == id) {
                    self.stage(NodeOp::Update(id, node), &sender);
                }
            }
            NodesMsg::ImportFromUrl(uri) => match v2ray_rs_subscription::parse_uri(&uri) {
                Ok(node) => {
                    if let Err(err) = node.validate() {
                        let _ = sender.output(NodesOutput::Notice(format!("Invalid node: {err}")));
                        return;
                    }
                    self.stage(NodeOp::Add(ManualNode::new(node)), &sender);
                }
                Err(err) => {
                    let _ =
                        sender.output(NodesOutput::Notice(format!("Failed to parse URI: {err}")));
                }
            },
            NodesMsg::ResetStorage => {
                self.stage(NodeOp::Reset, &sender);
            }
            NodesMsg::SaveFinished(result) => {
                match result {
                    Ok(committed) => {
                        self.draft.complete((*committed).clone());
                        self.nodes = self.draft.display();
                        self.load_error = None;
                        committed_changed = true;
                    }
                    Err(err) => {
                        log::error!("save manual nodes: {err}");
                        // This edit returns to the last durable list. Later
                        // edits are still queued and settle on their own
                        // receipts, so one failed batch is reported once.
                        self.draft.fail();
                        self.nodes = self.draft.display();
                        if !self.failure_reported {
                            self.failure_reported = true;
                            report = Some(format!("Failed to save manual nodes: {err}"));
                        }
                    }
                }
                if self.draft.is_empty() {
                    self.failure_reported = false;
                }
            }
            NodesMsg::SetActiveNode(active) => {
                if self.active_node == active {
                    return;
                }
                self.active_node = active;
            }
        }

        // Only the durable list decides whether a connectable node exists:
        // an in-flight edit must not flip the app's connected-state gate.
        let has_active = self.draft.committed_active();
        let _ = sender.output(NodesOutput::ActiveNodesChanged(has_active));
        if committed_changed {
            let _ = sender.output(NodesOutput::NodesChanged);
        }
        if let Some(message) = report {
            let _ = sender.output(NodesOutput::Notice(message));
        }
        render_list(&self.nodes, &self.list_container, &sender, self.active_node);
    }
}

impl NodesPage {
    /// Applies an edit to the display and enqueues it immediately.
    ///
    /// Enqueue order is submission order, and the same order is kept for
    /// receipts, so a Quit barrier enqueued after this edit covers it. The
    /// edit cannot wait for an earlier receipt: staging it outside the writer
    /// FIFO would let that barrier overtake it.
    fn stage(&mut self, op: NodeOp, sender: &ComponentSender<Self>) {
        op.clone().apply(&mut self.nodes);
        self.draft.stage(op.clone());
        let write = match op {
            NodeOp::Reset => NodeWrite::Reset(self.store.enqueue_reset_manual_nodes()),
            edit => NodeWrite::Edit(
                self.store
                    .enqueue_update_manual_nodes(move |nodes| edit.apply(nodes)),
            ),
        };
        let sender = sender.clone();
        relm4::gtk::glib::spawn_future_local(async move {
            let result = match write {
                NodeWrite::Edit(receipt) => match receipt.await {
                    Ok(Ok(((), nodes))) => Ok(nodes),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => Err(WRITER_STOPPED.to_string()),
                },
                NodeWrite::Reset(receipt) => match receipt.await {
                    Ok(Ok(nodes)) => Ok(nodes),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => Err(WRITER_STOPPED.to_string()),
                },
            };
            sender.input(NodesMsg::SaveFinished(result));
        });
    }
}

fn render_list(
    nodes: &[ManualNode],
    container: &gtk::ListBox,
    sender: &ComponentSender<NodesPage>,
    active_node: Option<Uuid>,
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
        let node_row = build_node_row(node, sender, active_node == Some(node.id));
        container.append(&node_row);
    }
}

fn build_node_row(
    node: &ManualNode,
    sender: &ComponentSender<NodesPage>,
    active: bool,
) -> adw::ActionRow {
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

    if active {
        let connected = gtk::Label::builder()
            .label("Connected")
            .css_classes(["caption", "success"])
            .valign(gtk::Align::Center)
            .tooltip_text("Currently connected node")
            .build();
        row.add_suffix(&connected);
    }

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

    let connect_btn = gtk::Button::builder()
        .label("Connect")
        .has_frame(false)
        .sensitive(node.enabled)
        .build();
    {
        let id = node.id;
        let s = sender.clone();
        let p = popover.clone();
        connect_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(NodesMsg::ConnectNode(id));
        });
    }

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

    popover_box.append(&connect_btn);
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
    let error_label = gtk::Label::builder()
        .css_classes(["error"])
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .build();
    content.append(&error_label);
    dialog.set_extra_child(Some(&content));

    {
        let dialog = dialog.clone();
        let error_label = error_label.clone();
        let update_validity = {
            let dialog = dialog.clone();
            let error_label = error_label.clone();
            let form = form.clone();
            move || match form.validation_error() {
                Some(message) => {
                    error_label.set_text(message);
                    error_label.set_visible(true);
                    dialog.set_response_enabled("save", false);
                }
                None => {
                    error_label.set_visible(false);
                    dialog.set_response_enabled("save", true);
                }
            }
        };
        update_validity();
        for entry in form.validation_entries() {
            let update_validity = update_validity.clone();
            entry.connect_changed(move |_| update_validity());
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_core::persistence;

    #[test]
    fn grpc_multi_mode_row_notes_sing_box_and_keeps_value() {
        crate::gtk_test::run(|| {
            let rows = TransportRows::new(&TransportSettings::Grpc(GrpcSettings {
                service_name: "svc".into(),
                multi_mode: true,
            }));

            assert_eq!(rows.grpc_multi_mode.title(), "gRPC Multi Mode");
            assert_eq!(
                rows.grpc_multi_mode.subtitle(),
                Some("Not used by sing-box".into())
            );
            assert!(rows.grpc_multi_mode.is_sensitive());
            match rows.value() {
                TransportSettings::Grpc(grpc) => {
                    assert_eq!(grpc.service_name, "svc");
                    assert!(grpc.multi_mode);
                }
                other => panic!("expected gRPC transport, got {other:?}"),
            }
        });
    }

    #[test]
    fn spider_x_row_notes_sing_box_and_keeps_value() {
        crate::gtk_test::run(|| {
            let rows = TlsRows::new(Some(&TlsSettings {
                reality: true,
                public_key: Some("pk".into()),
                spider_x: Some("/spx".into()),
                ..Default::default()
            }));

            assert_eq!(
                rows.spider_x.title(),
                "Reality Spider X (not used by sing-box)"
            );
            assert!(rows.spider_x.is_sensitive());
            let tls = rows.value().expect("TLS stays enabled");
            assert_eq!(tls.spider_x.as_deref(), Some("/spx"));
        });
    }

    #[test]
    fn ws_transport_round_trip_preserves_custom_headers() {
        crate::gtk_test::run(|| {
            let mut headers = HashMap::new();
            headers.insert("User-Agent".into(), "custom-agent/1.0".into());
            headers.insert("X-Edge".into(), "token".into());

            let initial = ProxyNode::Vless(VlessConfig {
                address: "edge.example.com".into(),
                port: 443,
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                encryption: Some("none".into()),
                flow: None,
                transport: TransportSettings::Ws(WsSettings {
                    path: "/ws".into(),
                    host: Some("cdn.example.com".into()),
                    headers: headers.clone(),
                }),
                tls: None,
                remark: Some("Before".into()),
            });

            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let form = NodeForm::from_node(&initial, &content);
            if let NodeForm::Vless {
                common, transport, ..
            } = &form
            {
                common.remark.set_text("After");
                transport.ws_path.set_text("/ws-v2");
            } else {
                panic!("expected VLESS form");
            }

            let saved = form.to_node();
            match saved {
                ProxyNode::Vless(cfg) => {
                    assert_eq!(cfg.remark.as_deref(), Some("After"));
                    match cfg.transport {
                        TransportSettings::Ws(ws) => {
                            assert_eq!(ws.path, "/ws-v2");
                            assert_eq!(ws.host.as_deref(), Some("cdn.example.com"));
                            assert_eq!(ws.headers, headers);
                        }
                        other => panic!("expected Ws transport, got {other:?}"),
                    }
                }
                other => panic!("expected Vless node, got {other:?}"),
            }
        });
    }

    fn manual_node(remark: &str) -> ManualNode {
        ManualNode::new(ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "203.0.113.7".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some(remark.into()),
        }))
    }

    #[test]
    fn submitted_node_edits_settle_until_their_receipts() {
        let a = manual_node("a");
        let b = manual_node("b");
        let mut draft = NodeDraft::default();
        draft.seed(vec![a.clone()]);

        draft.stage(NodeOp::Toggle(a.id));
        draft.stage(NodeOp::Add(b.clone()));

        let display = draft.display();
        assert_eq!(display.len(), 2);
        assert!(!display[0].enabled);
        assert_eq!(display[1].id, b.id);
        assert!(
            matches!(draft.front(), Some(NodeOp::Toggle(id)) if id == a.id),
            "the toggle is the oldest pending write"
        );

        draft.complete(vec![ManualNode {
            enabled: false,
            ..a.clone()
        }]);
        assert_eq!(draft.display().len(), 2);
        assert!(
            matches!(draft.front(), Some(NodeOp::Add(_))),
            "the add still waits for its own receipt"
        );

        draft.complete(vec![
            ManualNode {
                enabled: false,
                ..a.clone()
            },
            b,
        ]);
        assert!(draft.is_empty());
        assert_eq!(draft.display().len(), 2);
    }

    #[test]
    fn failed_receipt_rolls_back_only_its_own_edit() {
        let a = manual_node("a");
        let b = manual_node("b");
        let mut draft = NodeDraft::default();
        draft.seed(vec![a.clone()]);
        draft.stage(NodeOp::Delete(a.id));
        draft.stage(NodeOp::Add(b.clone()));

        draft.fail();

        assert!(
            !draft.is_empty(),
            "the later edit was already enqueued and keeps its receipt"
        );
        assert_eq!(draft.display(), vec![a.clone(), b.clone()]);
        assert!(
            matches!(draft.front(), Some(NodeOp::Add(node)) if node.id == b.id),
            "the failed delete is retired; the add is next in receipt order"
        );

        // The failed delete never reached the store, so the add transaction
        // runs against the still-durable list and its receipt carries `a`.
        // Adopting that receipt keeps the rolled-back node visible instead of
        // hiding a durable entry.
        draft.complete(vec![a.clone(), b.clone()]);
        assert!(draft.is_empty());
        assert_eq!(
            draft.display(),
            vec![a, b],
            "the add receipt adopts the durable list, including the rolled-back delete"
        );
    }

    #[test]
    fn committed_activity_ignores_pending_edits() {
        let a = manual_node("a");
        let mut draft = NodeDraft::default();
        draft.seed(vec![a.clone()]);
        assert!(draft.committed_active());

        draft.stage(NodeOp::Toggle(a.id));
        assert!(
            draft.committed_active(),
            "an unacknowledged toggle does not change the durable active state"
        );

        draft.complete(vec![ManualNode {
            enabled: false,
            ..a
        }]);
        assert!(!draft.committed_active());
    }

    #[test]
    fn staged_reset_clears_the_list_in_submission_order() {
        let a = manual_node("a");
        let b = manual_node("b");
        let mut draft = NodeDraft::default();
        draft.seed(vec![a.clone()]);
        draft.stage(NodeOp::Add(b.clone()));
        draft.stage(NodeOp::Reset);

        assert!(draft.display().is_empty());

        draft.complete(vec![a, b]);
        assert!(matches!(draft.front(), Some(NodeOp::Reset)));
        assert!(draft.display().is_empty());

        draft.complete(Vec::new());
        assert!(draft.is_empty());
        assert!(draft.display().is_empty());
    }

    #[tokio::test]
    async fn queued_node_receipts_settle_in_submission_order() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = v2ray_rs_core::persistence::AppPaths::for_profile_in(
            v2ray_rs_core::profile::AppProfile::Test,
            tmp.path(),
        );
        paths.ensure_dirs().unwrap();
        let store = WorkspaceStore::new(paths.clone());
        let a = manual_node("a");
        let b = manual_node("b");

        store
            .enqueue_save_manual_nodes(vec![a.clone()])
            .await
            .unwrap()
            .unwrap();
        // Both edits are enqueued before either receipt is consumed: this is
        // the page's submission order, and a barrier queued behind them must
        // observe both.
        let first = store.enqueue_update_manual_nodes({
            let id = a.id;
            move |nodes| NodeOp::Toggle(id).apply(nodes)
        });
        let second = store.enqueue_update_manual_nodes({
            let node = b.clone();
            move |nodes| NodeOp::Add(node).apply(nodes)
        });
        let barrier = store.enqueue_barrier();

        let (_, committed) = first.await.unwrap().unwrap();
        assert_eq!(committed.len(), 1);
        assert!(!committed[0].enabled);

        let (_, committed) = second.await.unwrap().unwrap();
        assert_eq!(committed.len(), 2);
        assert!(!committed[0].enabled);
        assert_eq!(committed[1].id, b.id);

        barrier.await.unwrap().unwrap();
        assert_eq!(persistence::load_manual_nodes(&paths).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn node_write_receipts_deliver_the_durable_list() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = v2ray_rs_core::persistence::AppPaths::for_profile_in(
            v2ray_rs_core::profile::AppProfile::Test,
            tmp.path(),
        );
        paths.ensure_dirs().unwrap();
        let store = WorkspaceStore::new(paths.clone());
        let node = manual_node("durable");

        store
            .enqueue_save_manual_nodes(vec![node.clone()])
            .await
            .unwrap()
            .unwrap();
        let (_, committed) = store
            .enqueue_update_manual_nodes({
                let id = node.id;
                move |nodes| NodeOp::Toggle(id).apply(nodes)
            })
            .await
            .unwrap()
            .unwrap();
        assert!(!committed[0].enabled);

        let committed = store.enqueue_reset_manual_nodes().await.unwrap().unwrap();
        assert!(committed.is_empty());
        assert!(!paths.custom_nodes_path().exists());
    }
}
