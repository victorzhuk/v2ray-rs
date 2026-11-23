//! `v2ray-rs-antidpi-probe` — test-only rawsend probe: sends a single
//! TCP/UDP packet through the raw socket path so the netns harness can watch
//! it with tcpdump. Not part of the shipped CLI surface.

use clap::Parser;
use std::net::Ipv4Addr;
use v2ray_rs_antidpi::packet::{OutBuilder, TCP_FLAG_ACK, TCP_FLAG_PSH};
use v2ray_rs_antidpi::rawsend::RawSender;

const MAX_IP_PACKET: usize = 65535;

fn parse_hex_packet(hex: &str) -> Result<Vec<u8>, String> {
    let h: String = hex.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if h.is_empty() {
        return Err("empty hex packet".into());
    }
    if !h.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid hex digit".into());
    }
    if h.len() / 2 > MAX_IP_PACKET {
        return Err(format!(
            "packet exceeds IPv4 limit of {MAX_IP_PACKET} bytes"
        ));
    }
    let mut out = Vec::with_capacity(h.len() / 2);
    for i in (0..h.len()).step_by(2) {
        out.push(
            u8::from_str_radix(&h[i..i + 2], 16).map_err(|e| format!("invalid hex digit: {e}"))?,
        );
    }
    Ok(out)
}

fn probe_payload(size: usize, udp: bool) -> Result<Vec<u8>, String> {
    let overhead: usize = if udp { 28 } else { 40 };
    if size.checked_add(overhead).is_none_or(|t| t > MAX_IP_PACKET) {
        return Err(format!(
            "payload plus {} bytes of headers exceeds IPv4 packet limit of {MAX_IP_PACKET}",
            overhead
        ));
    }
    Ok((0..size).map(|i| (i % 251) as u8).collect())
}

#[derive(Parser)]
struct Args {
    /// Destination IPv4 address.
    #[arg(long)]
    dst: String,
    /// Destination port.
    #[arg(long, default_value_t = 443)]
    port: u16,
    /// Source port.
    #[arg(long, default_value_t = 54321)]
    sport: u16,
    /// Send UDP instead of TCP.
    #[arg(long)]
    udp: bool,
    /// fwmark to set as decimal or 0x-hex (0 disables).
    #[arg(long, default_value = "1073741824")]
    mark: String,
    /// Payload size in bytes.
    #[arg(long, default_value_t = 32)]
    size: usize,
    /// Wait for one queued packet (queue 200) and send the probes while its
    /// verdict is still pending, before accepting it.
    #[arg(long)]
    hold_queue: bool,
    /// Send an exact packet given as hex (overrides the built packet).
    #[arg(long = "bytes-hex")]
    bytes_hex: Option<String>,
}

fn main() {
    let args = Args::parse();
    let dst: Ipv4Addr = args.dst.parse().expect("dst ip");
    if args.hold_queue {
        hold_queue_and_send(&args, dst);
        return;
    }
    let mark = args
        .mark
        .strip_prefix("0x")
        .map(|h| u32::from_str_radix(h, 16))
        .unwrap_or_else(|| args.mark.parse())
        .expect("mark value");
    let sender = RawSender::new(mark).expect("raw sender");
    let out = if let Some(hex) = &args.bytes_hex {
        let bytes = match parse_hex_packet(hex) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        };
        v2ray_rs_antidpi::packet::OutPacket::from_bytes(bytes)
    } else {
        let payload = match probe_payload(args.size, args.udp) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        };
        if args.udp {
            match OutBuilder::udp([10, 205, 0, 2], dst.octets(), args.sport, args.port)
                .payload(payload)
                .ttl(64)
                .build()
            {
                Ok(out) => out,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            }
        } else {
            match OutBuilder::tcp([10, 205, 0, 2], dst.octets(), args.sport, args.port)
                .seq(12345)
                .ack(54321)
                .flags(TCP_FLAG_PSH | TCP_FLAG_ACK)
                .window(64240)
                .payload(payload)
                .ttl(64)
                .build()
            {
                Ok(out) => out,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            }
        }
    };
    for i in 0..3 {
        match sender.send(&out.bytes, dst) {
            Ok(n) => println!("sent #{i}: {n} bytes"),
            Err(e) => {
                println!("send error: {e}");
                std::process::exit(1);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// Receives one queued packet and raw-sends probes while the verdict is
/// pending — isolates whether a pending NFQUEUE verdict suppresses rawsend.
fn hold_queue_and_send(args: &Args, dst: Ipv4Addr) {
    use nfq::{Queue, Verdict};
    let sender = RawSender::new(0x40000000).expect("raw sender");
    let payload = match probe_payload(args.size, false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let out = match OutBuilder::tcp([10, 205, 0, 2], dst.octets(), args.sport, args.port)
        .seq(12345)
        .ack(54321)
        .flags(TCP_FLAG_PSH | TCP_FLAG_ACK)
        .payload(payload)
        .ttl(64)
        .build()
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let mut queue = Queue::open().expect("queue open");
    queue.bind(200).expect("queue bind");
    queue.set_nonblocking(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut got_one = false;
    while std::time::Instant::now() < deadline {
        match queue.recv() {
            Ok(msg) => {
                println!("queued packet received; sending probes while pending");
                for i in 0..3 {
                    match sender.send(&out.bytes, dst) {
                        Ok(n) => println!("sent #{i}: {n} bytes"),
                        Err(e) => println!("send error: {e}"),
                    }
                }
                let mut msg = msg;
                msg.set_verdict(Verdict::Accept);
                queue.verdict(msg).expect("verdict");
                got_one = true;
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    if !got_one {
        println!("no queued packet arrived within deadline");
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_packet_roundtrip() {
        let b = parse_hex_packet("4500 0028").unwrap();
        assert_eq!(b, vec![0x45, 0x00, 0x00, 0x28]);
    }

    #[test]
    fn hex_packet_rejects_bad_input() {
        assert!(parse_hex_packet("").is_err());
        assert!(parse_hex_packet("   ").is_err());
        assert!(parse_hex_packet("450").is_err());
        assert!(parse_hex_packet("zz").is_err());
        assert!(parse_hex_packet("4g").is_err());
    }

    #[test]
    fn hex_packet_rejects_oversized() {
        let big = "00".repeat(MAX_IP_PACKET + 1);
        assert!(parse_hex_packet(&big).is_err());
        let max = "00".repeat(MAX_IP_PACKET);
        assert!(parse_hex_packet(&max).is_ok());
    }

    #[test]
    fn probe_payload_bounds() {
        assert!(probe_payload(MAX_IP_PACKET - 40, false).is_ok());
        assert!(probe_payload(MAX_IP_PACKET - 39, false).is_err());
        assert!(probe_payload(usize::MAX - 1, false).is_err());
        assert!(probe_payload(MAX_IP_PACKET - 28, true).is_ok());
        assert!(probe_payload(MAX_IP_PACKET - 27, true).is_err());
    }
}
