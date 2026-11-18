use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use v2ray_rs_core::models::SubscriptionSource;
use v2ray_rs_subscription::{
    FetchError, SubscriptionError, SubscriptionService, UpdateError, subscription_url_is_public,
};

fn install_crypto_provider() {
    static RUSTLS_PROVIDER: Once = Once::new();
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// One-shot loopback HTTP server answering with a single-node URI list and
/// counting every accepted TCP connection.
fn serve_subscription_once() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = connections.clone();
    std::thread::spawn(move || {
        let Some(stream) = listener.incoming().next() else {
            return;
        };
        counted.fetch_add(1, Ordering::SeqCst);
        let Ok(mut stream) = stream else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let body = "vless://fixture-uuid@node.example.com:443#Loopback\r\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });
    (format!("http://{addr}/sub"), connections)
}

#[tokio::test]
async fn production_service_refuses_private_initial_url_without_connecting() {
    install_crypto_provider();
    let (url, connections) = serve_subscription_once();

    let service = SubscriptionService::new();
    let err = service
        .add_and_fetch("Loopback".to_string(), SubscriptionSource::Url { url })
        .await
        .unwrap_err();

    match err {
        SubscriptionError::Update(UpdateError::Fetch(FetchError::InvalidUrl(msg))) => {
            assert!(msg.contains("public host"), "unexpected refusal: {msg}");
        }
        other => panic!("expected non-transient target refusal, got {other:?}"),
    }
    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "production fetch must not open a TCP connection to a private target"
    );
}

#[tokio::test]
async fn loopback_fixture_opt_in_fetches_private_url() {
    install_crypto_provider();
    let (url, connections) = serve_subscription_once();

    let service = SubscriptionService::new_allowing_private_targets();
    let outcome = service
        .add_and_fetch("Loopback".to_string(), SubscriptionSource::Url { url })
        .await
        .expect("loopback fixture opt-in must fetch");

    assert_eq!(outcome.subscription.nodes.len(), 1);
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "the fixture server must serve exactly one connection"
    );
}

#[test]
fn public_predicate_rejects_private_literals_and_localhost() {
    for url in [
        "http://127.0.0.1:8080/sub",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]:8080/sub",
        "http://[::ffff:10.0.0.1]/sub",
        "https://localhost/sub",
        "https://sub.localhost/sub",
    ] {
        assert!(
            !subscription_url_is_public(url),
            "private target accepted: {url}"
        );
    }
    assert!(
        subscription_url_is_public("https://example.com/sub"),
        "public target refused"
    );
    assert!(
        !subscription_url_is_public("ftp://example.com/sub"),
        "non-HTTP scheme accepted"
    );
}
