# Real Delay Privacy Note

When you run a Real Delay test, the configured test URL (default: `https://www.gstatic.com/generate_204`) is reached **through each tested proxy node**, exactly as real traffic would be. This applies to all supported backends (sing-box, xray, v2ray).

This means:
- The proxy server operator can see the test request in their traffic logs.
- The test URL host sees a request originating from the proxy server's IP.
- This is the intended behavior — it verifies the full proxy chain works end-to-end.

You can change the test URL in **Preferences → Real Delay**. Common alternatives:
- `https://cp.cloudflare.com/generate_204`
- `https://www.apple.com/library/test/success.html`

The test URL is never contacted directly from your machine; all requests route through the proxy.