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
}
