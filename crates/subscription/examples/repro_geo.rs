//! Reproduction: parse typical share links per protocol/transport and dump the
//! generated outbound for each backend, so protocol-specific emission bugs show
//! up as diffs between the share link and the generated JSON.

use v2ray_rs_core::config::{ConfigGenerator, SingboxGenerator, V2rayGenerator, XrayGenerator};
use v2ray_rs_core::models::{AppSettings, BackendType, ProxyNode};
use v2ray_rs_subscription::parse_uri;

const LINKS: &[(&str, &str)] = &[
    (
        "reality-vision",
        "vless://11111111-2222-3333-4444-555555555555@example.com:443?\
         type=tcp&security=reality&fp=chrome&pbk=SbVKOEMjK0sIlbwg4akyBg5mL5KZwwB-ed4eEE7YnRc&\
         sid=6ba85179&sni=yahoo.com&flow=xtls-rprx-vision&encryption=none#Reality",
    ),
    (
        "vless-ws-tls",
        "vless://11111111-2222-3333-4444-555555555555@cdn.example.com:443?\
         type=ws&path=%2Fwspath&host=ws.example.com&security=tls&\
         sni=ws.example.com&fp=chrome&alpn=h2%2Chttp%2F1.1&encryption=none#VlessWsTls",
    ),
    (
        "vless-ws-tls-insecure",
        "vless://11111111-2222-3333-4444-555555555555@cdn.example.com:443?\
         type=ws&path=%2Fwspath&host=ws.example.com&security=tls&\
         sni=ws.example.com&fp=chrome&allowInsecure=1&encryption=none#VlessWsTlsInsecure",
    ),
    (
        "vmess-ws-tls",
        "vmess://eyJhZGQiOiJjZG4uZXhhbXBsZS5jb20iLCJwb3J0IjoiNDQzIiwiaWQiOiIxMTExMTExMS0yMjIyLTMzMzMtNDQ0NC01NTU1NTU1NTU1NTUiLCJhaWQiOjAsIm5ldCI6IndzIiwicGF0aCI6Ii93c3BhdGgiLCJob3N0Ijoid3MuZXhhbXBsZS5jb20iLCJ0bHMiOiJ0bHMiLCJzbmkiOiJ3cy5leGFtcGxlLmNvbSIsInBzIjoiVk1lc3NXcyJ9",
    ),
    (
        "trojan-tls",
        "trojan://trojanpass@example.com:443?security=tls&sni=example.com&fp=chrome#TrojanTls",
    ),
    (
        "vless-xhttp-tls",
        "vless://11111111-2222-3333-4444-555555555555@example.com:443?\
         type=xhttp&path=%2Fxh&host=example.com&security=tls&sni=example.com&fp=chrome&encryption=none#XhttpTls",
    ),
    (
        "vless-tcp-tls",
        "vless://11111111-2222-3333-4444-555555555555@example.com:443?\
         type=tcp&security=tls&sni=example.com&fp=chrome&flow=xtls-rprx-vision&encryption=none#TcpTls",
    ),
];

fn main() {
    let settings = AppSettings::default();
    for (kind, generator) in [
        (BackendType::Xray, &XrayGenerator as &dyn ConfigGenerator),
        (BackendType::V2ray, &V2rayGenerator),
        (BackendType::SingBox, &SingboxGenerator),
    ] {
        println!("\n================ backend = {kind:?} ================");
        for (name, link) in LINKS {
            let node: ProxyNode = match parse_uri(link) {
                Ok(n) => n,
                Err(e) => {
                    println!("[{name}] PARSE ERROR: {e}");
                    continue;
                }
            };
            if let Err(e) = node.validate() {
                println!("[{name}] VALIDATE ERROR: {e}");
                continue;
            }
            match generator.generate(std::slice::from_ref(&node), &[], &settings) {
                Ok(cfg) => {
                    let ob = &cfg["outbounds"][0];
                    println!(
                        "[{name}] outbound:\n{}",
                        serde_json::to_string_pretty(ob).unwrap()
                    );
                }
                Err(e) => println!("[{name}] GENERATE ERROR: {e}"),
            }
        }
    }
}
