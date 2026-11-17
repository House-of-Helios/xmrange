use xmrange_core::trial::{is_settled, score, SettleRule, Tick};

#[test]
fn real_5_thread_lpm_window_scores_like_the_python_runner() {
    let ticks: Vec<Tick> = include_str!("fixtures/lpm-5t-measure.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let s = score(&ticks, 25.0).unwrap();
    assert!((s.hs - 1825.0).abs() / 1825.0 < 0.02, "hs {}", s.hs);
    assert!((s.sys_w.unwrap() - 9.10).abs() < 0.2, "sys {:?}", s.sys_w);
    assert!((s.hw_sys.unwrap() - 200.5).abs() < 5.0, "hw_sys {:?}", s.hw_sys);
    assert!(s.hw_cpu.unwrap() > 400.0);
    assert!(s.hw_sys_ci95.unwrap() < 5.0);
    assert!(!s.noisy);
    assert!(is_settled(&ticks, &SettleRule::default()), "a measure window is settled");
}
