//! "Save diagnostics": a plain-text report written only when the user presses the button, to a
//! file they pick in a save dialog. Nothing is ever sent anywhere (doctrine §2: no telemetry).
//! It holds no personal data:
//! - no wallet, not even masked: any line carrying an address, full or masked, is dropped;
//! - no IPs (`redact::redact_ips`);
//! - no home path, user name, computer name or worker name: each is replaced by a placeholder.
use std::fmt::Write;

/// The report's facts, gathered by the app. The tape lines are the ones the app shows on screen.
pub struct Facts<'a> {
    pub version: &'a str,
    pub macos: Option<String>,
    pub chip: Option<&'a str>,
    /// The hardware model identifier (`hw.model`, e.g. "Macmini9,1"): a type, not a name.
    pub model: Option<String>,
    pub pool: &'a str,
    pub tor: bool,
    pub node: Option<crate::node::Node>,
    /// The cost card's inputs: "0.16 $/kwh · period day" (the card itself is mean whole-mac watts
    /// × 24 h × this rate × the period's days).
    pub cost_inputs: String,
    /// What runs: the held throttle's setup while holding, else the user's own ("8 threads · utility").
    pub running: String,
    pub optimizer_mode: &'a str,
    pub trials: Option<u64>,
    pub low_power: Option<bool>,
    pub tape: &'a [String],
    /// p2pool mode: p2pool's own last output lines (`p2pool::recent_output`).
    pub p2pool_output: &'a [String],
    /// The release audit's live scans (`audit::report`): counts, ports and yes/no only.
    pub privacy: &'a [String],
    /// The optimizer's state and every measured setup (`trial_lines`).
    pub trials_table: &'a [String],
    /// This session's hourly mining lines (the engine's "hour" tape lines, kept by the app).
    pub hours: &'a [String],
    /// Words that name the person or the Mac (user name, computer and host names, the worker
    /// name), replaced wherever they appear, ignoring case.
    pub private: &'a [String],
    /// The home folder, shown as "~" wherever it appears.
    pub home: Option<&'a str>,
}

/// Whether `line` carries a wallet address: a full one (95 or 106 base58 characters starting
/// with 4 or 8) or the app's own mask (`47C4…zPB`).
fn has_address(line: &str) -> bool {
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let full = line.split(|c: char| !B58.contains(c)).any(|t| (t.len() == 95 || t.len() == 106) && (t.starts_with('4') || t.starts_with('8')));
    full || line.contains('…')
}

/// Replaces every case-insensitive occurrence of `word` in `s`.
fn replace_ci(s: &str, word: &str, with: &str) -> String {
    if word.is_empty() {
        return s.to_string();
    }
    let (lower, w) = (s.to_lowercase(), word.to_lowercase());
    if lower.len() != s.len() {
        return s.replace(word, with); // non-ASCII case folding changed lengths: exact match only
    }
    // Whole words only: a short name must not cut into other words ("minutes" became
    // "m[private]tes" in a 0.2.21 report). A word edge is any character that isn't a letter or digit.
    let edge = |c: Option<char>| c.map_or(true, |c| !c.is_alphanumeric());
    let mut out = String::new();
    let mut i = 0;
    let mut from = 0;
    while let Some(k) = lower[from..].find(&w) {
        let at = from + k;
        let end = at + w.len();
        if edge(lower[..at].chars().next_back()) && edge(lower[end..].chars().next()) {
            out.push_str(&s[i..at]);
            out.push_str(with);
            i = end;
        }
        from = end;
    }
    out.push_str(&s[i..]);
    out
}

/// One line for the report: dropped if it carries an address; otherwise IPs redacted, the home
/// folder shown as "~", and every private word replaced.
pub fn scrub(line: &str, private: &[String], home: Option<&str>) -> Option<String> {
    if has_address(line) {
        return None;
    }
    let mut s = crate::redact::redact_ips(line);
    if let Some(h) = home.filter(|h| h.len() > 1) {
        s = s.replace(h, "~");
    }
    let mut words: Vec<&String> = private.iter().filter(|w| w.len() >= 3).collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    for w in words {
        s = replace_ci(&s, w, "[private]");
    }
    Some(s)
}

/// The optimizer's state, then one line per measured setup (complete, scored; the newest of each),
/// with both denominators, so a report shows why a profile picked what it holds.
pub fn trial_lines(prefs: &crate::optimizer::OptimizerPrefs, measure: &str, recs: &[crate::ledger::TrialRecord], profile_log: &[String]) -> Vec<String> {
    let name = crate::optimizer::profile_name(prefs.profile);
    let mut out = vec![format!(
        "profile {name} · measure {measure} · mode {:?} · holding {}",
        prefs.mode,
        prefs.holding.as_ref().map_or("nothing".to_string(), |p| p.setup.label())
    )
    .to_lowercase()];
    // Where the profile came from: every pill press is logged; anything else is flagged.
    // 0.2.16 logged the names low · medium · high.
    let old = match name {
        "quiet" => "low",
        "balanced" => "medium",
        _ => "high",
    };
    match profile_log.last() {
        Some(last) if last.contains(&format!(" · {name} · ")) || last.contains(&format!(" · {old} · ")) => out.push(format!("profile set by: {last}")),
        Some(last) => out.push(format!("PROFILE CHANGED WITHOUT A PILL: last pill was {last}")),
        None if name == "balanced" => out.push("profile set by: the default (no pill pressed)".into()),
        None => out.push(format!("PROFILE IS {name} WITH NO PILL PRESS LOGGED (logging began in 0.2.16; it came from settings.json)")),
    }
    let mut seen = std::collections::HashSet::new();
    for r in recs.iter().filter(|r| !r.partial) {
        let Some(s) = &r.score else { continue };
        let label = crate::ledger::trial_label(&r.config);
        if !seen.insert(label.clone()) {
            continue;
        }
        let hw = |w: Option<f64>| w.map_or("n/a".to_string(), |w| format!("{w:.1}"));
        out.push(format!(
            "{label} · {:.0} h/s · whole mac {} w ({} h/w) · cpu {:.1} w ({} h/w){}",
            s.hs,
            hw(s.sys_w),
            hw(s.hw_sys),
            s.cpu_w,
            hw(s.hw_cpu),
            if s.noisy { " · noisy" } else { "" }
        ));
    }
    out
}

pub fn report(f: &Facts) -> String {
    let mut out = String::new();
    let yn = |b: Option<bool>| b.map_or("unknown", |v| if v { "on" } else { "off" });
    let _ = writeln!(out, "XMRange diagnostics · no wallet, no IPs, no names");
    let _ = writeln!(out, "version {}", f.version);
    let _ = writeln!(out, "macOS {}", f.macos.as_deref().unwrap_or("unknown"));
    let model = f.model.as_deref().unwrap_or("unknown");
    let named = crate::macs::lookup(model).map_or(String::new(), |m| format!(" ({})", m.name));
    let _ = writeln!(out, "chip {} · model {model}{named}", f.chip.unwrap_or("unknown"));
    let _ = writeln!(out, "pool {} · tor {}", f.pool, if f.tor { "on" } else { "off" });
    let _ = writeln!(out, "node {}", f.node.as_ref().map_or("not checked".to_string(), |n| format!("{n:?}")));
    let _ = writeln!(out, "running {} · optimizer {} · low power {}", f.running, f.optimizer_mode, yn(f.low_power));
    let _ = writeln!(out, "cost inputs {}", f.cost_inputs);
    let _ = writeln!(out, "trials {}", f.trials.map_or("unknown".to_string(), |n| n.to_string()));
    if !f.privacy.is_empty() {
        let _ = writeln!(out, "\n── privacy check ──");
        for l in f.privacy.iter().filter_map(|l| scrub(l, f.private, f.home)) {
            let _ = writeln!(out, "{l}");
        }
    }
    if !f.trials_table.is_empty() {
        let _ = writeln!(out, "\n── trials ──");
        for l in f.trials_table {
            let _ = writeln!(out, "{l}");
        }
    }
    if !f.hours.is_empty() {
        let _ = writeln!(out, "\n── by hour ({}) ──", f.hours.len());
        for l in f.hours.iter().filter_map(|l| scrub(l, f.private, f.home)) {
            let _ = writeln!(out, "{l}");
        }
    }
    let _ = writeln!(out, "\n── tape ({} lines, scrubbed) ──", f.tape.len());
    for l in f.tape.iter().filter_map(|l| scrub(l, f.private, f.home)) {
        let _ = writeln!(out, "{l}");
    }
    if !f.p2pool_output.is_empty() {
        let _ = writeln!(out, "\n── p2pool output ({} lines, scrubbed) ──", f.p2pool_output.len());
        for l in f.p2pool_output.iter().filter_map(|l| scrub(l, f.private, f.home)) {
            let _ = writeln!(out, "{l}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_word_is_replaced_only_as_a_whole_word() {
        let private = vec!["inu".to_string(), "Helios-iMac".to_string()];
        assert_eq!(scrub("connecting to tor · a first start can take a few minutes", &private, None).unwrap(), "connecting to tor · a first start can take a few minutes");
        assert_eq!(scrub("worker inu on Helios-iMac.local", &private, None).unwrap(), "worker [private] on [private].local");
        assert_eq!(scrub("INU set", &private, None).unwrap(), "[private] set", "any case");
    }

    #[test]
    fn the_profile_says_where_it_came_from() {
        use crate::optimizer::{OptimizerPrefs, Profile};
        let low = OptimizerPrefs { profile: Profile::Light, ..Default::default() };
        let lines = |p: &OptimizerPrefs, log: &[String]| trial_lines(p, "whole mac", &[], log)[1].clone();
        assert_eq!(lines(&low, &["2026-10-03 21:14:05 · quiet · pill".into()]), "profile set by: 2026-10-03 21:14:05 · quiet · pill");
        assert_eq!(lines(&low, &["2026-10-03 20:01:00 · low · pill".into()]), "profile set by: 2026-10-03 20:01:00 · low · pill", "0.2.16's name");
        assert!(lines(&low, &["2026-10-03 21:14:05 · balanced · pill".into()]).starts_with("PROFILE CHANGED WITHOUT A PILL"));
        assert!(lines(&low, &[]).starts_with("PROFILE IS quiet WITH NO PILL PRESS LOGGED"));
        assert_eq!(lines(&OptimizerPrefs::default(), &[]), "profile set by: the default (no pill pressed)");
    }

    const W: &str = "47C4vQftcmvKEmz6NQPsjc3LYnP3Rk5kZW1YD7PT3crbKrXaEUvg42QZ9vXM2Czk5bhoWLkF9wjJiMyMYUBCZYeRGjDhzPB";

    #[test]
    fn no_wallet_and_no_ip_survive() {
        let tape = vec![
            format!("12:00 net use pool gulf.moneroocean.stream:20004 203.0.113.9 as {W}"),
            "12:01 miner wallet 47C4…zPB".to_string(),
            "12:02 net connected to 203.0.113.9:20004".to_string(),
            "12:03 cpu accepted (1/0) diff 40000".to_string(),
            "12:04 error p2pool exited: using /Users/helios/Library/Application Support/XMRange/p2pool".to_string(),
            "12:05 miner worker Helios-M3Air set on Helios's MacBook Air".to_string(),
        ];
        let private = vec!["helios".to_string(), "Helios's MacBook Air".to_string(), "helios-m3air".to_string()];
        let r = report(&Facts {
            version: "0.2.1", macos: Some("15.6".into()), chip: Some("Apple M3"), model: Some("Mac15,12".into()), pool: "p2pool", tor: true,
            node: None, cost_inputs: "0.16 $/kwh · period day".into(), running: "6 cores".into(), optimizer_mode: "manual", trials: Some(12), low_power: Some(false), tape: &tape,
            privacy: &["sockets: xmrig · listens local:3620 · local 3640 · remote none".to_string()],
            trials_table: &["profile balanced · measure sys · mode hold · holding 4 cores".to_string()],
            hours: &["03:00:00 hour past hour · 1297 h/s · whole mac 24.1 w (53.8 h/w) · cpu 6.0 w (216.2 h/w)".to_string()],
            p2pool_output: &[format!("2026-10-2 20:41:30.1 P2Pool wallet = {W}"), "2026-10-2 20:41:31.2 StratumServer new connection from 127.0.0.1:52144".to_string()],
            private: &private, home: Some("/Users/helios"),
        });
        assert!(!r.contains(W) && !r.contains("47C4") && !r.contains('…'), "no wallet, not even masked");
        assert!(!r.contains("203.0.113.9"), "no IPs");
        assert!(!r.to_lowercase().contains("helios"), "no user, computer or worker name, any case: {r}");
        assert!(r.contains("~/Library/Application Support/XMRange/p2pool"), "the home folder shows as ~");
        assert!(r.contains("accepted (1/0)") && r.contains("12:02 net connected to"), "everything else stays");
        assert!(r.contains("pool p2pool · tor on") && r.contains("trials 12") && r.contains("cost inputs 0.16 $/kwh · period day"));
        assert!(r.contains("── trials ──\nprofile balanced") && r.contains("── by hour (1) ──\n03:00:00 hour past hour · 1297 h/s"), "{r}");
        assert!(r.contains("── p2pool output (2 lines, scrubbed) ──") && r.contains("StratumServer new connection from"), "{r}");
    }
}
