use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub algo: Option<String>,
    #[serde(default)]
    pub uptime: u64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub hashrate: Hashrate,
    #[serde(default)]
    pub results: Results,
    #[serde(default)]
    pub connection: Connection,
}

/// xmrig's pool connection: how many times a logged-in connection was lost (a login that never
/// succeeds isn't counted). Its `ip` field is the pool's address and is never read.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Connection {
    #[serde(default)]
    pub failures: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Hashrate {
    #[serde(default)]
    pub total: Vec<Option<f64>>,
    #[serde(default)]
    pub highest: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Results {
    #[serde(default)]
    pub shares_good: u64,
    #[serde(default)]
    pub shares_total: u64,
    #[serde(default)]
    pub diff_current: u64,
}

impl Summary {
    /// 10-second average hashrate, if xmrig has one yet.
    pub fn hs10(&self) -> Option<f64> {
        self.hashrate.total.first().copied().flatten()
    }

    /// 60-second average hashrate, if xmrig has one yet (its first minute has none).
    pub fn hs60(&self) -> Option<f64> {
        self.hashrate.total.get(1).copied().flatten()
    }

    /// 15-minute average hashrate, if xmrig has one yet (its first 15 minutes have none).
    pub fn hs15(&self) -> Option<f64> {
        self.hashrate.total.get(2).copied().flatten()
    }
}

/// Loopback client for xmrig's HTTP API. The app is the config's source of truth:
/// it only ever PUTs config and never GETs it (GET /2/config would return the wallet).
pub struct XmrigApi {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl XmrigApi {
    pub fn new(port: u16, token: &str) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).build();
        Self { base: format!("http://127.0.0.1:{port}"), token: token.to_string(), agent }
    }

    pub fn summary(&self) -> Result<Summary> {
        let resp = self
            .agent
            .get(&format!("{}/2/summary", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(map_err)?;
        Ok(resp.into_json::<Summary>()?)
    }

    pub fn put_config(&self, config: &serde_json::Value) -> Result<()> {
        self.agent
            .put(&format!("{}/2/config", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_string(&config.to_string())
            .map_err(map_err)?;
        Ok(())
    }

    fn json_rpc(&self, method: &str) -> Result<()> {
        self.agent
            .post(&format!("{}/json_rpc", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_string(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method}).to_string())
            .map_err(map_err)?;
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.json_rpc("pause")
    }

    pub fn resume(&self) -> Result<()> {
        self.json_rpc("resume")
    }
}

fn map_err(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, _) => anyhow!("xmrig api returned HTTP {code}"),
        other => anyhow!("xmrig api unreachable: {other}"),
    }
}
