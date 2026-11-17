/// Replace IPv4 addresses with "ip hidden" (doctrine §3: IPs never persisted).
pub fn redact_ipv4(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut copied) = (0usize, 0usize);
    while i < b.len() {
        let boundary = i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.');
        if boundary && b[i].is_ascii_digit() {
            if let Some(end) = match_ipv4(b, i) {
                out.push_str(&s[copied..i]);
                out.push_str("ip hidden");
                i = end;
                copied = end;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[copied..]);
    out
}

fn match_ipv4(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    for part in 0..4 {
        let d0 = i;
        while i < b.len() && b[i].is_ascii_digit() && i - d0 < 3 {
            i += 1;
        }
        if i == d0 || (i < b.len() && b[i].is_ascii_digit()) {
            return None;
        }
        if part < 3 {
            if i < b.len() && b[i] == b'.' {
                i += 1;
            } else {
                return None;
            }
        }
    }
    if i < b.len() && (b[i] == b'.' || b[i].is_ascii_digit()) {
        return None;
    }
    Some(i)
}

/// Replace IPv6 addresses with "ip hidden". A candidate is a run of hex digits, ':' and '.'
/// that isn't glued to a word on either side (so `core::miner`, hashes and timestamps stay),
/// holds at least two ':' and parses as an IPv6 address, allowing a trailing '.' (sentence end)
/// or ":port" to stay outside.
pub fn redact_ipv6(s: &str) -> String {
    let b = s.as_bytes();
    let part = |c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.';
    let glued = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'.';
    let mut out = String::with_capacity(s.len());
    let (mut i, mut copied) = (0usize, 0usize);
    while i < b.len() {
        if part(b[i]) && (i == 0 || !glued(b[i - 1])) {
            let mut end = i;
            while end < b.len() && part(b[end]) {
                end += 1;
            }
            let free_after = end == b.len() || !(b[end].is_ascii_alphanumeric() || b[end] == b'_');
            if free_after {
                if let Some(len) = ipv6_len(&s[i..end]) {
                    out.push_str(&s[copied..i]);
                    out.push_str("ip hidden");
                    i += len;
                    copied = i;
                    continue;
                }
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    out.push_str(&s[copied..]);
    out
}

fn ipv6_len(run: &str) -> Option<usize> {
    let trimmed = run.trim_end_matches('.');
    let mut cands = vec![run, trimmed];
    if let Some(k) = trimmed.rfind(':') {
        if k > 0 && k + 1 < trimmed.len() && trimmed[k + 1..].bytes().all(|c| c.is_ascii_digit()) {
            cands.push(&trimmed[..k]);
        }
    }
    // A bare "::" (or "::." before trimming) has no hex digit at all: it names no address.
    cands
        .into_iter()
        .find(|c| c.bytes().any(|b| b.is_ascii_hexdigit()) && c.matches(':').count() >= 2 && c.parse::<std::net::Ipv6Addr>().is_ok())
        .map(str::len)
}

/// Both families: IPv6 first, so an IPv4-mapped address (`::ffff:1.2.3.4`) goes whole.
pub fn redact_ips(s: &str) -> String {
    redact_ipv4(&redact_ipv6(s))
}

/// Rewrites a dev log with every IP hidden, atomically (temp file + rename, same mode).
/// Streams line by line (a `BufReader` in, a `BufWriter` to the temp file out), so a log that grew
/// for an entire mining session is never held whole in memory; redaction runs per line, so a line
/// boundary never splits an address. Ok(false) when there was nothing to hide (the file isn't
/// touched, and any temp file made while checking is removed).
#[cfg(feature = "dev-diagnostics")]
pub fn redact_file_in_place(path: &std::path::Path) -> anyhow::Result<bool> {
    use std::io::{BufRead, BufReader, BufWriter, Write};
    if std::env::var("XMRANGE_DEV_UNREDACTED").as_deref() == Ok("1") {
        return Ok(false);
    }
    let tmp = path.with_extension("log.redact-tmp");
    let stream = || -> anyhow::Result<bool> {
        let reader = BufReader::new(std::fs::File::open(path)?);
        let mut writer = BufWriter::new(std::fs::File::create(&tmp)?);
        let mut changed = false;
        for line in reader.lines() {
            let line = line?;
            let clean = redact_ips(&line);
            changed |= clean != line;
            writer.write_all(clean.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(changed)
    };
    let changed = match stream() {
        Ok(changed) => changed,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if !changed {
        let _ = std::fs::remove_file(&tmp);
        return Ok(false);
    }
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(true)
}

/// Redacts every `*.log` in `dir`. Call it only when no xmrig of ours can be writing there
/// (after the port-3620 check). Returns how many files were rewritten.
#[cfg(feature = "dev-diagnostics")]
pub fn sweep_dev_logs(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .filter(|p| redact_file_in_place(p).unwrap_or(false))
        .count()
}

#[cfg(test)]
mod tests {
    use super::{redact_ips, redact_ipv4, redact_ipv6};

    #[test]
    fn hides_ipv4_and_keeps_everything_else() {
        assert_eq!(
            redact_ipv4("use pool gulf.moneroocean.stream:20004  205.172.58.170"),
            "use pool gulf.moneroocean.stream:20004  ip hidden"
        );
        assert_eq!(redact_ipv4("peer 1.2.3.4:5555 ok"), "peer ip hidden:5555 ok");
        for keep in [
            "accepted (93/0) diff 45401 (104 ms)",
            "speed 10s/60s/15m 1826.6 1855.7 n/a H/s",
            "XMRig/6.26.0-mo5 clang/17.0.0",
            "1.2.3.4.5 is not an address",
            "1234.2.3.4 neither",
            "°F 131 · ünïcode ✓",
        ] {
            assert_eq!(redact_ipv4(keep), keep);
        }
    }

    #[test]
    fn hides_ipv6_in_its_common_shapes() {
        for (raw, want) in [
            ("use pool [2001:db8::1]:20004", "use pool [ip hidden]:20004"),
            ("peer 2606:4700:4700::1111 ok", "peer ip hidden ok"),
            ("fe80:0:0:0:200:f8ff:fe21:67cf", "ip hidden"),
            ("mapped ::ffff:205.172.58.170 here", "mapped ip hidden here"),
            ("loopback ::1", "loopback ip hidden"),
            ("from 2001:db8::1.", "from ip hidden."),
            ("upper 2001:DB8::1 here", "upper ip hidden here"),
            ("stratum+tcp://[2001:db8::1]:3333", "stratum+tcp://[ip hidden]:3333"),
        ] {
            assert_eq!(redact_ipv6(raw), want, "{raw}");
        }
    }

    #[test]
    fn a_bare_double_colon_names_no_address() {
        assert_eq!(redact_ipv6("range -- :: -- end"), "range -- :: -- end");
    }

    #[test]
    fn keeps_what_only_looks_like_ipv6() {
        for keep in [
            "[2026-09-27 12:34:56.789] net use pool gulf.moneroocean.stream:20004",
            "speed 10s/60s/15m 1826.6 1855.7 n/a H/s",
            "xmrange_core::miner::launch",
            "job a1b2c3d4 diff 45401 algo rx/0",
            "mac aa:bb:cc:dd:ee:ff",
            "ratio 3:2 · 12:30",
            "°F 131 · ünïcode ✓",
            "started at 12:34 sharp",
        ] {
            assert_eq!(redact_ipv6(keep), keep);
        }
    }

    #[test]
    fn redact_ips_hides_both_families() {
        assert_eq!(redact_ips("a 205.172.58.170 b [2001:db8::1]:3333"), "a ip hidden b [ip hidden]:3333");
    }

    /// A zone id (`%en0`) names a local interface, not an address: the address itself is hidden,
    /// and the `%...` suffix (glued by `%`, not by the char classes `redact_ipv6` matches on) stays.
    #[test]
    fn hides_the_address_ahead_of_a_zone_id() {
        assert_eq!(redact_ipv6("iface fe80::1%en0 ok"), "iface ip hidden%en0 ok");
        assert_eq!(redact_ipv6("peer [fe80::1%en0]:3333 ok"), "peer [ip hidden%en0]:3333 ok");
    }
}
