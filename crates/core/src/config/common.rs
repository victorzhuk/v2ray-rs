use crate::models::ProxyNode;

pub(crate) fn outbound_tag(node: &ProxyNode, index: usize) -> String {
    match node.remark() {
        Some(name) if !name.is_empty() => format!("proxy-{index}-{name}"),
        _ => format!("proxy-{index}"),
    }
}
