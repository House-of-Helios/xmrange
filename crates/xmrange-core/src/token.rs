/// 128-bit random hex token from the OS RNG (per-launch xmrig API secret, handoff dir names).
pub fn new_token() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("OS random number generator unavailable");
    b.iter().map(|x| format!("{x:02x}")).collect()
}
