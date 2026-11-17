//! house-report: sends ONE report that the user has read and approved to the House.
//!
//! The report text comes on stdin. It is wrapped as a NIP-17 private message (NIP-59 gift wrap,
//! NIP-44 encryption) from a throwaway key made for this report alone, addressed to the House's
//! public key, and published to public Nostr relays through a SOCKS5 proxy (the app's own Arti).
//! The relays see only ciphertext from a key used once, from a Tor exit. Only the House key can
//! read it. There is no direct mode: without a loopback --socks it refuses.
//!
//! Usage: house-report --to <hex pubkey> --app <name> --version <v> [--kind crash|problem|idea]
//!                     --socks 127.0.0.1:<port>   < report.md
//! Prints {"sent":n,"relays":[...]} and exits 0 when at least one relay took it, 1 when none did,
//! 2 on bad input.
use nostr::nips::nip59::GiftWrapBuilder;
use nostr_sdk::prelude::*;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Probed 2026-11-17: each takes a kind-1059 write through Tor and returns it to the recipient.
const RELAYS: &[&str] = &["wss://nos.lol", "wss://relay.primal.net", "wss://relay.snort.social", "wss://nostr.mom"];
/// Bigger than any honest report; keeps a stuck pipe or a pasted log from going out.
const MAX_BODY: usize = 64 * 1024;
/// Relays may drop the wrap after this; the House reader polls hourly.
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
const KINDS: &[&str] = &["crash", "problem", "idea"];

#[derive(Debug, PartialEq)]
struct Args {
    to: PublicKey,
    app: String,
    version: String,
    kind: String,
    socks: SocketAddr,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let get = |name: &str| {
        argv.iter().position(|a| a == name).and_then(|i| argv.get(i + 1)).cloned()
    };
    let to = get("--to").ok_or("--to <hex pubkey> is required")?;
    if to.len() != 64 || !to.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("--to must be a 64-character hex public key".into());
    }
    let to = PublicKey::from_hex(&to).map_err(|e| format!("--to: {e}"))?;
    let word = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    let app = get("--app").filter(|s| word(s)).ok_or("--app <name> is required (letters, digits, . _ -)")?;
    let version = get("--version").filter(|s| word(s)).ok_or("--version <v> is required")?;
    let kind = get("--kind").unwrap_or_else(|| "problem".into());
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("--kind must be one of {KINDS:?}"));
    }
    let socks: SocketAddr = get("--socks").ok_or("--socks 127.0.0.1:<port> is required (never direct)")?
        .parse().map_err(|_| "--socks must be ip:port")?;
    if !socks.ip().is_loopback() {
        return Err("--socks must be a loopback address (the app's own Tor)".into());
    }
    Ok(Args { to, app, version, kind, socks })
}

/// The sealed content: a small JSON envelope the House reader checks before it keeps anything.
fn envelope(a: &Args, body: &str) -> String {
    serde_json::json!({ "v": 1, "app": a.app, "version": a.version, "kind": a.kind, "body": body }).to_string()
}

fn wrap(a: &Args, body: &str, sender: &Keys) -> Result<Event, String> {
    let rumor = EventBuilder::new(Kind::PrivateDirectMessage, envelope(a, body))
        .tag(Tag::public_key(a.to))
        .finalize_unsigned(sender.public_key());
    GiftWrapBuilder::new(a.to, rumor).expiration(KEEP).finalize(sender).map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let args = match parse(&argv) {
        Ok(a) => a,
        Err(e) => { eprintln!("house-report: {e}"); std::process::exit(2) }
    };
    let mut raw = Vec::new();
    let read = tokio::io::stdin().take(MAX_BODY as u64 + 1).read_to_end(&mut raw).await;
    let body = match (read, String::from_utf8(raw)) {
        (Ok(_), Ok(b)) if b.len() > MAX_BODY => { eprintln!("house-report: report is over {MAX_BODY} bytes"); std::process::exit(2) }
        (Ok(_), Ok(b)) if b.trim().is_empty() => { eprintln!("house-report: empty report on stdin"); std::process::exit(2) }
        (Ok(_), Ok(b)) => b,
        _ => { eprintln!("house-report: stdin is not UTF-8 text"); std::process::exit(2) }
    };
    let sender = Keys::generate(); // used for this one report, then gone
    let gift = match wrap(&args, &body, &sender) {
        Ok(g) => g,
        Err(e) => { eprintln!("house-report: wrap: {e}"); std::process::exit(2) }
    };
    let client = Client::builder().proxy(Proxy::all(args.socks)).build();
    for url in RELAYS {
        let _ = client.add_relay(*url).await;
    }
    client.connect().await;
    let sent: Vec<String> = match tokio::time::timeout(Duration::from_secs(90), client.send_event(&gift)).await {
        // Count only relays that answered OK true: a bare "sent" is no proof it was stored.
        Ok(Ok(out)) => out.success.iter().filter(|(_, st)| matches!(st, EventSendStatus::Ack(_))).map(|(u, _)| u.to_string()).collect(),
        _ => Vec::new(),
    };
    client.shutdown().await;
    println!("{}", serde_json::json!({ "sent": sent.len(), "relays": sent }));
    std::process::exit(if sent.is_empty() { 1 } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip59::UnwrappedGift;

    fn argv(extra: &[&str]) -> Vec<String> {
        ["house-report"].iter().chain(extra).map(|s| s.to_string()).collect()
    }
    const HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn refuses_bad_input() {
        let ok = ["--to", HEX, "--app", "xmrange", "--version", "0.0.2", "--socks", "127.0.0.1:3640"];
        assert!(parse(&argv(&ok)).is_ok());
        assert!(parse(&argv(&ok[..6])).is_err(), "no --socks: never direct");
        let mut bad = ok; bad[7] = "192.0.2.1:9050";
        assert!(parse(&argv(&bad)).is_err(), "a non-loopback proxy");
        let mut bad = ok; bad[1] = "npub1xyz";
        assert!(parse(&argv(&bad)).is_err(), "--to must be hex");
        let mut bad = ok; bad[3] = "x m";
        assert!(parse(&argv(&bad)).is_err(), "--app is one word");
        let mut k = argv(&ok); k.extend(["--kind".into(), "rant".into()]);
        assert!(parse(&k).is_err());
    }

    #[test]
    fn only_the_house_key_reads_it_and_the_sender_is_throwaway() {
        let house = Keys::generate();
        let a = parse(&argv(&["--to", &house.public_key().to_hex(), "--app", "xmrange", "--version", "0.0.2",
            "--kind", "crash", "--socks", "127.0.0.1:3640"])).unwrap();
        let sender = Keys::generate();
        let gift = wrap(&a, "the trial log stopped at 7 cores", &sender).unwrap();
        assert_eq!(gift.kind, Kind::GiftWrap);
        assert_ne!(gift.pubkey, sender.public_key(), "the outer key is random, not even the throwaway");
        assert!(!gift.content.contains("trial"), "ciphertext only");
        assert!(gift.tags.expiration().is_some());
        let g = UnwrappedGift::from_gift_wrap(&house, &gift).unwrap();
        let v: serde_json::Value = serde_json::from_str(&g.rumor.content).unwrap();
        assert_eq!((v["app"].as_str(), v["kind"].as_str(), v["body"].as_str()),
            (Some("xmrange"), Some("crash"), Some("the trial log stopped at 7 cores")));
        assert!(UnwrappedGift::from_gift_wrap(&Keys::generate(), &gift).is_err(), "another key can't read it");
    }
}
