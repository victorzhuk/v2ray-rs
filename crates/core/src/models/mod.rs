mod dns;
mod manual_node;
mod presets;
mod proxy;
mod resolve;
mod routing;
mod settings;
mod subscription;
mod validation;

pub use dns::{
    DnsConfig, DnsProtocol, DnsProviderPreset, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy,
    DnsValidationError, FakeIpConfig, HostOverride, builtin_dns_presets,
};
pub use manual_node::ManualNode;
pub use presets::{Preset, builtin_presets};
pub use proxy::{
    GrpcSettings, H2Settings, ProxyNode, ProxyNodeValidationError, ShadowsocksConfig, TlsSettings,
    TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings,
};
pub use resolve::{
    AutoResolveStrategy, ConnectionMetadata, ConnectionNodeRef, LastSuccessMetadata, LatencySample,
};
pub use routing::{RoutingRule, RoutingRuleSet, RuleAction, RuleMatch};
pub use settings::{
    AppSettings, BackendConfig, BackendType, Language, RealDelayCapability, RealDelaySettings,
};
pub use subscription::{Subscription, SubscriptionNode, SubscriptionSource};
pub use validation::{
    ValidationError, validate_country_code, validate_domain_pattern, validate_geosite_category,
    validate_ip_cidr, validate_rule_match, validate_test_url,
};
