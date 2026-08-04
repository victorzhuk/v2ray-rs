mod dns;
mod imported_profile;
mod manual_node;
mod presets;
mod proxy;
mod resolve;
mod routing;
mod settings;
mod subscription;
mod tun;
mod validation;

pub use dns::{
    DnsConfig, DnsProtocol, DnsProviderPreset, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy,
    DnsValidationError, FakeIpConfig, HostOverride, builtin_dns_presets,
};
pub use imported_profile::{ImportedProfile, resolve_effective_config};
pub use manual_node::ManualNode;
pub use presets::{Preset, builtin_presets};
pub use proxy::{
    GrpcSettings, H2Settings, ProxyNode, ProxyNodeValidationError, ShadowsocksConfig, TlsSettings,
    TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings, XhttpSettings,
};
pub use resolve::{
    AutoResolveStrategy, ConnectionMetadata, ConnectionNodeRef, LastSuccessMetadata, LatencySample,
};
pub use routing::{RoutingRule, RoutingRuleSet, RuleAction, RuleMatch};
pub use settings::{
    AppSettings, BackendConfig, BackendType, Language, RealDelayCapability, RealDelaySettings,
};
pub use subscription::{Subscription, SubscriptionNode, SubscriptionSource};
pub use tun::{DnsHijackMode, TunConfig, TunStack};
pub use validation::{
    ValidationError, validate_country_code, validate_domain_keyword, validate_domain_pattern,
    validate_geosite_category, validate_ip_cidr, validate_network_spec, validate_port_spec,
    validate_protocol_name, validate_rule_match, validate_test_url, validate_tun_interface_name,
};
