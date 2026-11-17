//! Problem reports: the text a user reads, edits and sends (doctrine §1d). Nothing here goes online;
//! the app hands the text to `house-report`, which seals it and sends it through Tor.
use std::fmt::Write;

/// XMRange's public report key. The private half lives only on the House's Mac.
pub const REPORT_PUBKEY: &str = "7ad24038f20035591e8ccc64ebddd76ae1e64ef50fa78db501f98b6e680f4ccd";
pub const KINDS: &[&str] = &["crash", "problem", "idea"];
/// house-report refuses more than 64 KB; this leaves room under it.
const MAX: usize = 60_000;

/// The report: header, the user's note, then the diagnostics text, scrubbed and capped.
pub fn build(kind: &str, version: &str, macos: &str, arch: &str, note: &str, diagnostics: &str, private: &[String], home: Option<&str>) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "XMRange {kind} report\nversion {version}\nmacOS {macos}\narch {arch}\nkind {kind}\n\n── note ──");
    let _ = writeln!(s, "{}\n\n{diagnostics}", note.trim());
    clean(&s, private, home)
}

/// Scrubs and caps any report text, including one the user edited: addresses dropped, IPs
/// redacted, every home folder shown as "~", private words replaced, oldest log lines cut first.
pub fn clean(text: &str, private: &[String], home: Option<&str>) -> String {
    let lines: Vec<String> = text
        .lines()
        .filter_map(|l| crate::diagnostics::scrub(l, private, home))
        .map(|l| other_homes(&l))
        .collect();
    cap(lines)
}

/// "/Users/anyone/x" becomes "~/x": a pasted path from any account, not only this one.
fn other_homes(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("/Users/") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 7..];
        let end = tail.find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'').unwrap_or(tail.len());
        out.push('~');
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Under the limit: log lines (everything after the first "── " heading) go oldest first.
fn cap(mut lines: Vec<String>) -> String {
    let size = |l: &[String]| l.iter().map(|x| x.len() + 1).sum::<usize>();
    let mut total = size(&lines);
    if let Some(start) = lines.iter().position(|l| l.starts_with("── ")) {
        let mut i = start + 1;
        while total > MAX && i < lines.len() {
            if lines[i].starts_with("── ") {
                i += 1;
            } else {
                total -= lines.remove(i).len() + 1;
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    if out.len() > MAX {
        let mut cut = MAX;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

// ── the crash file (doctrine §1d): one scrubbed file, written only by a panic ──────────────

/// The one file a crash leaves in the data folder. Deleted once the report is sent or dismissed, or after 7 days.
pub const CRASH_FILE: &str = "last-crash.txt";
const CRASH_MAX: usize = 16 * 1024;
const RING_MAX: usize = 200;
const CRASH_DAYS: u64 = 7;
pub const NO_CRASH_DETAILS: &str = "no crash details: XMRange was stopped without a panic (force quit, power loss, or the system ended it)";

/// The last engine tape lines, already scrubbed, plus the words that scrubbing uses.
struct Ring {
    lines: std::collections::VecDeque<String>,
    private: Vec<String>,
    home: Option<String>,
}
static RING: std::sync::Mutex<Ring> = std::sync::Mutex::new(Ring { lines: std::collections::VecDeque::new(), private: Vec::new(), home: None });

/// Sets the private words and home folder the ring scrubs with (called when an engine starts).
pub fn ring_scrubber(private: Vec<String>, home: Option<String>) {
    let mut r = RING.lock().unwrap_or_else(|e| e.into_inner());
    r.private = private;
    r.home = home;
}

/// Keeps one engine tape line, scrubbed, in memory only; the oldest beyond 200 fall off.
pub fn ring_push(line: &str) {
    let mut r = RING.lock().unwrap_or_else(|e| e.into_inner());
    let kept = clean(line, &r.private, r.home.as_deref());
    r.lines.push_back(kept.trim_end().to_string());
    while r.lines.len() > RING_MAX {
        r.lines.pop_front();
    }
}

/// The crash file's text: header, then the newest ring lines that fit, everything scrubbed, at most 16 KB.
pub fn crash_text(version: &str, message: &str, thread: &str, location: &str, lines: &[String], private: &[String], home: Option<&str>) -> String {
    let head = clean(&format!("XMRange crash\nversion {version}\nthread {thread}\nat {location}\npanic {message}\n── last tape lines ──\n"), private, home);
    let mut kept: Vec<&String> = Vec::new();
    let mut size = head.len();
    for l in lines.iter().rev() {
        if size + l.len() + 1 > CRASH_MAX {
            break;
        }
        size += l.len() + 1;
        kept.push(l);
    }
    let mut out = head;
    for l in kept.iter().rev() {
        out.push_str(l);
        out.push('\n');
    }
    // Scrubbed again whole (the ring's lines already were): the file never relies on one pass.
    let mut out = clean(&out, private, home);
    let mut cut = CRASH_MAX.min(out.len());
    while !out.is_char_boundary(cut) {
        cut -= 1;
    }
    out.truncate(cut);
    out
}

/// Writes `text` to `dir/last-crash.txt` (0600) through a temp file and a rename.
pub fn write_crash(dir: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = dir.join(format!("{CRASH_FILE}.tmp"));
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(text.as_bytes())?;
    drop(f);
    std::fs::rename(tmp, dir.join(CRASH_FILE))
}

/// Installs, once, a panic hook that writes the crash file and then runs the hook that was there.
pub fn install_crash_hook(dir: std::path::PathBuf, version: &'static str) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => s.to_string(),
            None => info.payload().downcast_ref::<String>().cloned().unwrap_or_default(),
        };
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        // try_lock: a panic while the ring is locked must not hang the hook.
        let (lines, private, home) = match RING.try_lock() {
            Ok(r) => (r.lines.iter().cloned().collect::<Vec<_>>(), r.private.clone(), r.home.clone()),
            Err(_) => (Vec::new(), Vec::new(), std::env::var("HOME").ok()),
        };
        let _ = write_crash(&dir, &crash_text(version, &message, &thread, &location, &lines, &private, home.as_deref()));
        prev(info);
    }));
}

/// At launch: a crash file is kept only when the last run really ended in a crash and it is under 7 days old.
pub fn tidy_crash(dir: &std::path::Path, crashed: bool) {
    let path = dir.join(CRASH_FILE);
    let old = std::fs::metadata(&path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.elapsed().ok()).is_none_or(|e| e.as_secs() > CRASH_DAYS * 86_400);
    if !crashed || old {
        let _ = std::fs::remove_file(path);
    }
}

/// The "before the crash" section of a crash report: the file's text, or why there is none.
pub fn before_the_crash(dir: &std::path::Path) -> String {
    let text = std::fs::read_to_string(dir.join(CRASH_FILE)).unwrap_or_else(|_| NO_CRASH_DETAILS.to_string());
    format!("── before the crash ──\n{}\n", text.trim_end())
}

/// The documented sample (docs/site/examples): fixed inputs that deliberately carry a wallet, IPs,
/// a home path, a computer name and a worker name, run through the real report pipeline. Returns
/// (the raw lines as the app saw them, the exact text house-report would receive). A test keeps
/// the committed files equal to this, so the docs cannot drift from the code.
#[cfg(any(test, feature = "dev-diagnostics"))]
pub fn sample() -> (String, String) {
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let wallet = format!("4{}", B58.chars().cycle().take(94).collect::<String>());
    let note = format!("the hashrate fell after sleep.\nmy wallet is\n{wallet}\nI am on 203.0.113.7 (2001:db8:85a3::8a2e:370:7334), computer Someones-MacBook-Pro.local");
    let tape: Vec<String> = vec![
        format!("12:00:01 miner wallet {wallet}"),
        "12:00:02 net connected to 203.0.113.7:20004".into(),
        "12:00:03 net tor circuit via 2001:db8:85a3::8a2e:370:7334".into(),
        "12:00:04 miner worker someones-rig set on Someones-MacBook-Pro.local".into(),
        "12:00:05 error could not read /Users/someone/Library/Application Support/XMRange/ledger.sqlite".into(),
        "12:00:06 cpu accepted (41/0) diff 40000".into(),
    ];
    let hours = vec!["13:00:00 hour past hour · 1297 h/s · whole mac 24.1 w (53.8 h/w) · cpu 6.0 w (216.2 h/w)".to_string()];
    let private = vec!["Someones-MacBook-Pro".to_string(), "Someones-MacBook-Pro.local".to_string(), "someone".to_string(), "someones-rig".to_string()];
    let home = Some("/Users/someone");
    let diag = crate::diagnostics::report(&crate::diagnostics::Facts {
        version: "0.0.1", macos: Some("15.6".into()), chip: Some("Apple M3"), model: Some("Mac15,12".into()), pool: "moneroocean", tor: true,
        node: None, cost_inputs: "0.16 $/kwh · period day".into(), running: "6 cores".into(), optimizer_mode: "manual", trials: Some(12), low_power: Some(false),
        tape: &tape, p2pool_output: &[], privacy: &["sockets: xmrig · local 3640 · remote none".to_string()],
        trials_table: &["profile balanced · measure sys · mode hold · holding 4 cores".to_string()],
        hours: &hours, private: &private, home,
    });
    let sent = build("problem", "0.0.1", "15.6", "apple silicon", &note, &diag, &private, home);
    let input = format!("── note ──\n{note}\n\n── tape ──\n{}\n\n── by hour ──\n{}\n\n── from this Mac ──\nhome folder {}\ncomputer name Someones-MacBook-Pro.local\nworker name someones-rig\n", tape.join("\n"), hours.join("\n"), home.unwrap_or(""));
    (input, sent)
}

/// `cargo dev report-sample`: writes the two sample files into `dir`.
#[cfg(feature = "dev-diagnostics")]
pub fn write_sample(dir: &std::path::Path) -> std::io::Result<()> {
    let (input, sent) = sample();
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("report-input.txt"), input)?;
    std::fs::write(dir.join("report-sent.txt"), sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "47C4vQftcmvKEmz6NQPsjc3LYnP3Rk5kZW1YD7PT3crbKrXaEUvg42QZ9vXM2Czk5bhoWLkF9wjJiMyMYUBCZYeRGjDhzPB";

    #[test]
    fn nothing_personal_survives() {
        let private = vec!["Studio-Mac".to_string(), "rigworker".to_string()];
        let note = format!("it broke.\nwallet {W}\nmy ip is 203.0.113.9 and 2001:db8:85a3::8a2e:370:7334\nlog in /Users/someone/Library/x on Studio-Mac, worker rigworker");
        let diag = "version 0.0.1\n\n── tape (2 lines, scrubbed) ──\n12:00 net connected to 198.51.100.7:20004\n12:01 error at /Users/someone/Documents/a";
        let r = build("problem", "0.0.1", "15.6", "apple silicon", &note, diag, &private, Some("/Users/someone"));
        for bad in [W, "203.0.113.9", "198.51.100.7", "2001:db8", "someone", "Studio-Mac", "rigworker"] {
            assert!(!r.contains(bad), "{bad} survived: {r}");
        }
        assert!(r.starts_with("XMRange problem report\nversion 0.0.1\nmacOS 15.6\narch apple silicon\nkind problem"));
        assert!(r.contains("it broke.") && r.contains("12:00 net connected to"));
        assert!(r.contains("wallet [wallet removed]"), "the line stays, the address is marked: {r}");
    }

    #[test]
    fn a_users_own_ellipsis_survives_and_the_mask_does_not() {
        let real = crate::config::mask_wallet(W);
        let r = clean(&format!("it stalls… then recovers\nmining to {real} now\nold mask 47C4…zPB and odd ab…cd"), &[], None);
        assert!(r.contains("it stalls... then recovers"), "{r}");
        assert!(r.contains("mining to [wallet removed] now") && !r.contains("47C4") && !r.contains("hzPB"), "{r}");
        assert!(r.contains("old mask [wallet removed] and odd [wallet removed]"), "any glued … is taken for a mask: {r}");
        assert!(!r.contains('…'), "{r}");
    }

    #[test]
    fn over_the_limit_the_oldest_log_lines_go_first() {
        let mut diag = String::from("version 1\n\n── tape ──\n");
        for i in 0..8000 {
            diag.push_str(&format!("line {i:05} padding padding\n"));
        }
        let r = build("crash", "1", "15", "intel", "note", &diag, &[], None);
        assert!(r.len() <= MAX + 1 && r.contains("note") && r.contains("── tape ──") && r.contains("line 07999") && !r.contains("line 00000"));
    }

    #[test]
    fn the_docs_sample_is_what_the_code_sends() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/site/examples");
        let (input, sent) = sample();
        let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap_or_default();
        assert_eq!(read("report-input.txt"), input, "stale: run `cargo dev report-sample`");
        assert_eq!(read("report-sent.txt"), sent, "stale: run `cargo dev report-sample`");
        for bad in ["4123456789", "203.0.113.7", "2001:db8", "someone", "Someones", "Users/"] {
            assert!(!sent.contains(bad), "{bad} survived: {sent}");
        }
        assert!(input.contains("203.0.113.7") && input.contains("/Users/someone"), "the input really carries them");
    }

    #[test]
    fn the_crash_file_holds_no_wallet_ip_or_home_and_stays_small() {
        let wallet = format!("4{}", "A1b2".repeat(24).chars().take(94).collect::<String>());
        let lines: Vec<String> = (0..400).map(|i| format!("12:00:{i:02} net via 203.0.113.7 {wallet} /Users/someone/x padding padding padding padding")).collect();
        let text = crash_text("0.0.1", &format!("bad {wallet} at 203.0.113.7 in /Users/someone/Library"), "main", "/Users/someone/src/a.rs:9", &lines, &["someone".into()], Some("/Users/someone"));
        let dir = std::env::temp_dir().join(format!("xmr-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_crash(&dir, &text).unwrap();
        let back = std::fs::read_to_string(dir.join(CRASH_FILE)).unwrap();
        for bad in [&wallet[..12], "203.0.113.7", "someone", "/Users/"] {
            assert!(!back.contains(bad), "{bad} survived: {back}");
        }
        assert!(back.len() <= 16 * 1024 && back.contains("[wallet removed]") && back.contains("panic bad"));
        assert!(back.contains("12:00:99") || back.contains("12:00:"), "newest lines kept");
        assert!(before_the_crash(&dir).starts_with("── before the crash ──\nXMRange crash"));
        tidy_crash(&dir, false);
        assert!(!dir.join(CRASH_FILE).exists());
        assert!(before_the_crash(&dir).contains(NO_CRASH_DETAILS));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ring_is_bounded_and_scrubbed() {
        ring_scrubber(vec!["someone".into()], Some("/Users/someone".into()));
        for i in 0..300 {
            ring_push(&format!("t{i} 203.0.113.7 someone"));
        }
        let r = RING.lock().unwrap();
        assert_eq!(r.lines.len(), 200);
        assert!(r.lines.back().unwrap().starts_with("t299") && !r.lines.iter().any(|l| l.contains("203.0.113.7") || l.contains("someone")));
    }
}
