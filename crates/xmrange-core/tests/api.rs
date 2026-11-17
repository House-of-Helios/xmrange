mod common;
use common::MockXmrig;
use xmrange_core::api::{Summary, XmrigApi};

const SUMMARY: &str = r#"{"algo":"rx/0","hashrate":{"total":[1826.6,1855.7,null]},
  "results":{"shares_good":93,"diff_current":45401},
  "connection":{"pool":"gulf.moneroocean.stream:20004","ip":"205.172.58.170"}}"#;

#[test]
fn reads_summary_with_bearer_token() {
    let m = MockXmrig::start(SUMMARY);
    let s = XmrigApi::new(m.port, "secret-token").summary().unwrap();
    assert_eq!(s.hs10(), Some(1826.6));
    assert_eq!(s.results.shares_good, 93);
    assert_eq!(s.algo.as_deref(), Some("rx/0"));
    let r = &m.requests()[0];
    assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/2/summary"));
    assert_eq!(r.auth.as_deref(), Some("Bearer secret-token"));
}

#[test]
fn wrong_token_is_an_error_not_silence() {
    let m = MockXmrig::start(SUMMARY);
    let e = XmrigApi::new(m.port, "wrong").summary().unwrap_err().to_string();
    assert!(e.contains("401"), "{e}");
}

#[test]
fn unreachable_api_is_an_error() {
    let port = { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() };
    let e = XmrigApi::new(port, "secret-token").summary().unwrap_err().to_string();
    assert!(e.contains("unreachable"), "{e}");
}

#[test]
fn put_config_sends_json_body() {
    let m = MockXmrig::start(SUMMARY);
    XmrigApi::new(m.port, "secret-token")
        .put_config(&serde_json::json!({"cpu": {"rx": [-1, -1]}}))
        .unwrap();
    let r = &m.requests()[0];
    assert_eq!((r.method.as_str(), r.path.as_str()), ("PUT", "/2/config"));
    assert_eq!(r.content_type.as_deref(), Some("application/json"));
    assert_eq!(r.body, r#"{"cpu":{"rx":[-1,-1]}}"#);
    assert!(m.requests().iter().all(|r| !(r.method == "GET" && r.path.contains("config"))));
}

#[test]
fn empty_hashrate_is_none() {
    let m = MockXmrig::start(r#"{"hashrate":{"total":[null,null,null]}}"#);
    assert_eq!(XmrigApi::new(m.port, "secret-token").summary().unwrap().hs10(), None);
}

const FULL: &str = r#"{"algo":"rx/0","uptime":754,"paused":false,
  "hashrate":{"total":[1826.6,1855.7,null],"highest":1966.2},
  "results":{"shares_good":93,"shares_total":94,"diff_current":45401},
  "connection":{"pool":"gulf.moneroocean.stream:20004","ip":"205.172.58.170"}}"#;

#[test]
fn reads_extended_summary_fields() {
    let m = MockXmrig::start(FULL);
    let s = XmrigApi::new(m.port, "secret-token").summary().unwrap();
    assert_eq!(s.uptime, 754);
    assert!(!s.paused);
    assert_eq!(s.hashrate.highest, Some(1966.2));
    assert_eq!(s.results.shares_total, 94);
    assert_eq!(s.hs60(), Some(1855.7));
    assert_eq!(s.hs15(), None, "no 15-minute average in xmrig's first 15 minutes");
}

#[test]
fn the_third_total_is_the_15_minute_average() {
    let s: Summary = serde_json::from_str(r#"{"hashrate":{"total":[1826.6,1855.7,1840.2]}}"#).unwrap();
    assert_eq!((s.hs10(), s.hs60(), s.hs15()), (Some(1826.6), Some(1855.7), Some(1840.2)));
}

#[test]
fn pause_and_resume_use_json_rpc() {
    let m = MockXmrig::start(FULL);
    let api = XmrigApi::new(m.port, "secret-token");
    api.pause().unwrap();
    api.resume().unwrap();
    let r = m.requests();
    assert_eq!((r[0].method.as_str(), r[0].path.as_str()), ("POST", "/json_rpc"));
    assert!(r[0].body.contains("\"method\":\"pause\"") && r[1].body.contains("\"method\":\"resume\""));
    assert_eq!(r[0].auth.as_deref(), Some("Bearer secret-token"));
}
