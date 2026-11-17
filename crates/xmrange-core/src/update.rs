//! Updates from the project's GitHub releases (doctrine §1c, owner ruling 2026-11-16), which
//! replaced the local update folder in 0.0.1. Release builds only; a dev app never asks GitHub.
//!
//! Each release carries `appcast.json` (the feed): its version, notes, and for each kind of Mac a
//! zip, the zip's SHA-256, and an Ed25519 signature over "<version>\n<sha256>" made with the key
//! whose public half is `RELEASE_PUBKEY`. The order of checks (Bookkeeper's): the signature before
//! anything is downloaded, the SHA-256 after, unpack with `ditto` only then. A tampered zip, a zip
//! relabelled as another version, a foreign key and an unsigned feed are all refused.
//!
//! Every request goes through Tor (the shared Arti, like the price feed): GitHub sees a Tor exit,
//! never the Mac, and no DNS lookup leaves it. The feed carries no wallet, worker, hashrate or
//! Ledger data; the request carries none either.
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "House-of-Helios/xmrange";
/// The public half of the release signing key (`tools/release-sign.swift public`). The private half
/// is `_devdata/release-signing.key` on the owner's build Mac only; `tools/release.sh` checks its own
/// signature against this value before anything is uploaded.
pub const RELEASE_PUBKEY: &str = "bb7f78f14383cd0d97a98181281751b0346f03314860fb1f5a80e055d38c404d";
/// A read-only, single-repo GitHub token, compiled in only while the repo is private (stage 5 of
/// the github-publish skill): `tools/release.sh` sets it from `_devdata/github-read-token` and
/// refuses to once the repo is public. Never set in a public build.
const READ_TOKEN: Option<&str> = option_env!("XMRANGE_GITHUB_READ_TOKEN");
/// A zip larger than this is refused while it downloads (the 0.3.0 zips are 21–24 MB).
pub const MAX_ZIP: u64 = 200 * 1024 * 1024;
const MAX_FEED: u64 = 1024 * 1024;

/// This build's kind of Mac, in the feed's words.
pub fn this_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "arm64"
    }
}

/// A dotted version ("1.2.3") as comparable numeric parts. A non-numeric or missing segment
/// reads as 0, so a malformed string never panics and just sorts low.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .split('.')
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0))
        .collect()
}

/// True when `candidate` is strictly newer than `running` (dotted numeric compare).
pub fn version_newer(candidate: &str, running: &str) -> bool {
    parse_version(candidate) > parse_version(running)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub file: String,
    pub sha256: String,
    pub sig: String,
}

/// `appcast.json` as `tools/release.sh` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// By kind of Mac: "arm64", "x86_64".
    pub assets: BTreeMap<String, Asset>,
}

/// A verified offer: newer, for this kind of Mac, its signature good. `url` is where the zip is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub notes: String,
    pub asset: Asset,
    pub url: String,
}

/// The one zip name a release ships for a kind of Mac, so a feed can't point anywhere else.
pub fn expected_file(version: &str, arch: &str) -> String {
    format!("XMRange-{version}-{}.zip", if arch == "x86_64" { "intel" } else { "apple-silicon" })
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The Ed25519 signature `sig_hex` over "<version>\n<sha256>" verifies with `pub_hex`.
pub fn verify_sig(pub_hex: &str, version: &str, sha256: &str, sig_hex: &str) -> bool {
    let (Some(key), Some(sig)) = (unhex(pub_hex), unhex(sig_hex)) else { return false };
    let msg = format!("{version}\n{sha256}");
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(msg.as_bytes(), &sig).is_ok()
}

/// The feed's offer for this Mac, if every check passes: a plain dotted version newer than
/// `running`, an asset for `arch` with the one expected file name and a well-formed SHA-256, and
/// a signature that verifies with `pub_hex`. Checked before anything is downloaded.
pub fn offer(feed: &Feed, running: &str, arch: &str, pub_hex: &str, url: impl Fn(&str) -> Option<String>) -> Option<Offer> {
    let v = &feed.version;
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit() || b == b'.') || !version_newer(v, running) {
        return None;
    }
    let asset = feed.assets.get(arch)?;
    if asset.file != expected_file(v, arch) || !is_sha256_hex(&asset.sha256) {
        return None;
    }
    if !verify_sig(pub_hex, v, &asset.sha256, &asset.sig) {
        return None;
    }
    Some(Offer { version: v.clone(), notes: feed.notes.clone(), asset: asset.clone(), url: url(&asset.file)? })
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpacks `zip` into a fresh `dir` with `ditto` and returns `dir/XMRange.app`. The zip must hold
/// exactly one entry, a real directory named `XMRange.app` (no symlink, nothing beside it).
pub fn unpack(zip: &Path, dir: &Path) -> Result<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir)?;
    let ok = std::process::Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(zip).arg(dir).status()?.success();
    if !ok {
        bail!("could not unpack the update");
    }
    let names: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
    let app = dir.join("XMRange.app");
    let real_dir = std::fs::symlink_metadata(&app).map(|m| m.is_dir()).unwrap_or(false);
    if names.len() != 1 || !real_dir {
        bail!("the update zip does not hold just XMRange.app");
    }
    Ok(app)
}

// ── GitHub, through Tor ────────────────────────────────────────────────────────────────────────

fn agent(socks: &str) -> Result<ureq::Agent> {
    Ok(ureq::AgentBuilder::new()
        .proxy(ureq::Proxy::new(format!("socks5://{socks}"))?)
        .timeout_connect(Duration::from_secs(60))
        .timeout(Duration::from_secs(900))
        .user_agent("XMRange")
        // The token goes to api.github.com only; the redirect to the file host drops it.
        .redirect_auth_headers(ureq::RedirectAuthHeaders::Never)
        .build())
}

fn get(agent: &ureq::Agent, url: &str, accept: &str) -> Result<ureq::Response> {
    let mut req = agent.get(url).set("Accept", accept);
    if let (Some(t), true) = (READ_TOKEN, url.starts_with("https://api.github.com/")) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    req.call().map_err(|e| anyhow!("{}", crate::redact::redact_ips(&e.to_string())))
}

fn read_capped(resp: ureq::Response, max: u64) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    resp.into_reader().take(max + 1).read_to_end(&mut v)?;
    if v.len() as u64 > max {
        bail!("reply too large");
    }
    Ok(v)
}

/// The latest release's feed and a way to find its files. With a token (private repo): the
/// releases API. Without: the public `releases/latest/download` and `releases/download` links.
fn latest(agent: &ureq::Agent) -> Result<(Feed, BTreeMap<String, String>)> {
    if READ_TOKEN.is_some() {
        let rel: serde_json::Value =
            serde_json::from_slice(&read_capped(get(agent, &format!("https://api.github.com/repos/{REPO}/releases/latest"), "application/vnd.github+json")?, MAX_FEED)?)?;
        let mut urls = BTreeMap::new();
        for a in rel.get("assets").and_then(|a| a.as_array()).into_iter().flatten() {
            if let (Some(n), Some(u)) = (a.get("name").and_then(|v| v.as_str()), a.get("url").and_then(|v| v.as_str())) {
                urls.insert(n.to_string(), u.to_string());
            }
        }
        let feed_url = urls.get("appcast.json").context("the latest release has no appcast.json")?;
        let feed = serde_json::from_slice(&read_capped(get(agent, feed_url, "application/octet-stream")?, MAX_FEED)?)?;
        Ok((feed, urls))
    } else {
        let url = format!("https://github.com/{REPO}/releases/latest/download/appcast.json");
        let feed: Feed = serde_json::from_slice(&read_capped(get(agent, &url, "application/octet-stream")?, MAX_FEED)?)?;
        let mut urls = BTreeMap::new();
        for a in feed.assets.values() {
            urls.insert(a.file.clone(), format!("https://github.com/{REPO}/releases/download/v{}/{}", feed.version, a.file));
        }
        Ok((feed, urls))
    }
}

/// Asks GitHub (through Tor) for the latest release; `Some` only for a verified, newer offer.
pub fn check_github(running: &str, cancel: &dyn Fn() -> bool) -> Result<Option<Offer>> {
    let _lease = crate::tor::acquire(cancel)?;
    let agent = agent(&crate::tor::socks_addr())?;
    let (feed, urls) = latest(&agent)?;
    Ok(offer(&feed, running, this_arch(), RELEASE_PUBKEY, |f| urls.get(f).cloned()))
}

/// Downloads the offer's zip (through Tor) into `dir`, checks its SHA-256, unpacks it, and returns
/// the unpacked `XMRange.app`. The zip and the unpacked copy stay in `dir` for the installer.
pub fn fetch(offer: &Offer, dir: &Path, cancel: &dyn Fn() -> bool) -> Result<PathBuf> {
    std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir)?;
    let zip = dir.join(&offer.asset.file);
    {
        let _lease = crate::tor::acquire(cancel)?;
        let agent = agent(&crate::tor::socks_addr())?;
        let resp = get(&agent, &offer.url, "application/octet-stream")?;
        let mut out = std::fs::File::create(&zip)?;
        let n = std::io::copy(&mut resp.into_reader().take(MAX_ZIP + 1), &mut out)?;
        if n > MAX_ZIP {
            let _ = std::fs::remove_file(&zip);
            bail!("the update is larger than {} MB", MAX_ZIP >> 20);
        }
    }
    if sha256_file(&zip)? != offer.asset.sha256 {
        let _ = std::fs::remove_file(&zip);
        bail!("the update's checksum does not match its signed feed");
    }
    unpack(&zip, &dir.join("unpacked"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);
    fn scratch() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("update-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A test vector made by tools/release-sign.swift (CryptoKit) with a throwaway key: proves the
    // signer and this verifier (ring) agree on Ed25519 and on the signed message.
    const TV_PUB: &str = "236e9e8ac5d0b0c2ea61641393dd7ef457809ec3abb3d3a628ce95b753b85217";
    const TV_SIG: &str = "3a02a203dc8e896f1dfdbc5a692d891b4b9daf5d45c9975c79a77bf1e63be0b2ce4bd76674c447b0775926a8c87b2c7a4ce97d8e64de61188f9f9b33d683d108";
    fn tv_sha() -> String {
        "a".repeat(64)
    }
    fn tv_feed() -> Feed {
        let mut assets = BTreeMap::new();
        assets.insert("arm64".into(), Asset { file: "XMRange-0.0.2-apple-silicon.zip".into(), sha256: tv_sha(), sig: TV_SIG.into() });
        Feed { version: "0.0.2".into(), notes: "n".into(), assets }
    }
    fn url(f: &str) -> Option<String> {
        Some(format!("u/{f}"))
    }

    #[test]
    fn version_compare() {
        assert!(version_newer("0.2.0", "0.1.9"));
        assert!(version_newer("0.0.2", "0.0.1"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.0.2", "0.3.0"), "a 0.3.x app never takes 0.0.x");
        assert!(version_newer("0.1.10", "0.1.9"));
        assert_eq!(parse_version("0.1.0-beta"), vec![0, 1, 0]);
    }

    #[test]
    fn a_swift_signature_verifies_and_a_relabel_does_not() {
        assert!(verify_sig(TV_PUB, "0.0.2", &tv_sha(), TV_SIG));
        assert!(!verify_sig(TV_PUB, "0.0.3", &tv_sha(), TV_SIG), "relabelled version");
        assert!(!verify_sig(TV_PUB, "0.0.2", &"b".repeat(64), TV_SIG), "another zip");
        assert!(!verify_sig(RELEASE_PUBKEY, "0.0.2", &tv_sha(), TV_SIG), "foreign key");
        assert!(!verify_sig(TV_PUB, "0.0.2", &tv_sha(), ""), "unsigned");
        assert!(!verify_sig("zz", "0.0.2", &tv_sha(), TV_SIG));
    }

    #[test]
    fn offer_takes_only_a_signed_newer_build_for_this_mac() {
        let f = tv_feed();
        let o = offer(&f, "0.0.1", "arm64", TV_PUB, url).expect("a good offer");
        assert_eq!((o.version.as_str(), o.url.as_str()), ("0.0.2", "u/XMRange-0.0.2-apple-silicon.zip"));
        assert!(offer(&f, "0.0.2", "arm64", TV_PUB, url).is_none(), "not newer");
        assert!(offer(&f, "0.3.0", "arm64", TV_PUB, url).is_none(), "older than running");
        assert!(offer(&f, "0.0.1", "x86_64", TV_PUB, url).is_none(), "no asset for this Mac");
        assert!(offer(&f, "0.0.1", "arm64", RELEASE_PUBKEY, url).is_none(), "foreign key");
        assert!(offer(&f, "0.0.1", "arm64", TV_PUB, |_| None).is_none(), "no download link");

        let mut g = tv_feed();
        g.version = "0.0.3".into();
        g.assets.get_mut("arm64").unwrap().file = "XMRange-0.0.3-apple-silicon.zip".into();
        assert!(offer(&g, "0.0.1", "arm64", TV_PUB, url).is_none(), "relabelled version");

        for bad in ["../XMRange-0.0.2-apple-silicon.zip", "XMRange-0.0.2-intel.zip", "x.zip", ""] {
            let mut g = tv_feed();
            g.assets.get_mut("arm64").unwrap().file = bad.into();
            assert!(offer(&g, "0.0.1", "arm64", TV_PUB, url).is_none(), "{bad:?}");
        }
        let mut g = tv_feed();
        g.version = "0.0.2/../9".into();
        assert!(offer(&g, "0.0.1", "arm64", TV_PUB, url).is_none(), "odd version text");
        let mut g = tv_feed();
        g.assets.get_mut("arm64").unwrap().sha256 = tv_sha().to_uppercase();
        assert!(offer(&g, "0.0.1", "arm64", TV_PUB, url).is_none(), "sha not lowercase hex");
    }

    #[test]
    fn feed_json_round_trips() {
        let f = tv_feed();
        let back: Feed = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn sha256_of_a_file() {
        let d = scratch();
        fs::write(d.join("abc"), "abc").unwrap();
        assert_eq!(sha256_file(&d.join("abc")).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    fn zip_of(src: &Path, zip: &Path) {
        assert!(std::process::Command::new("/usr/bin/ditto").args(["-c", "-k"]).arg(src).arg(zip).status().unwrap().success());
    }

    #[test]
    fn unpack_takes_just_one_real_xmrange_app() {
        let d = scratch();
        let good = d.join("good");
        fs::create_dir_all(good.join("XMRange.app/Contents")).unwrap();
        zip_of(&good, &d.join("good.zip"));
        assert_eq!(unpack(&d.join("good.zip"), &d.join("out1")).unwrap(), d.join("out1/XMRange.app"));

        let extra = d.join("extra");
        fs::create_dir_all(extra.join("XMRange.app")).unwrap();
        fs::write(extra.join("other"), "x").unwrap();
        zip_of(&extra, &d.join("extra.zip"));
        assert!(unpack(&d.join("extra.zip"), &d.join("out2")).is_err(), "something beside the app");

        let link = d.join("link");
        fs::create_dir_all(&link).unwrap();
        std::os::unix::fs::symlink("/Applications", link.join("XMRange.app")).unwrap();
        zip_of(&link, &d.join("link.zip"));
        assert!(unpack(&d.join("link.zip"), &d.join("out3")).is_err(), "a symlink named XMRange.app");

        let other = d.join("other");
        fs::create_dir_all(other.join("Other.app")).unwrap();
        zip_of(&other, &d.join("other.zip"));
        assert!(unpack(&d.join("other.zip"), &d.join("out4")).is_err(), "another app");
    }

    /// Live, through the bundled Arti, against the real repo. Private repo: build with
    /// XMRANGE_GITHUB_READ_TOKEN set. `cargo devtest -p xmrange-core --lib update -- --ignored`.
    #[test]
    #[ignore]
    fn the_real_latest_release_verifies() {
        let o = check_github("0.0.0", &|| false).expect("github through tor").expect("an offer");
        println!("offer {} {}", o.version, o.asset.file);
        let d = scratch();
        let app = fetch(&o, &d, &|| false).expect("download, checksum, unpack");
        assert!(app.join("Contents/Info.plist").is_file());
    }
}
