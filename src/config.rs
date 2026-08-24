//! Configuration and endpoint selection.
//!
//! The same slskd instance is typically reachable two ways: fast on the LAN and
//! slower through a reverse proxy from outside. Rather than make the user pick,
//! `resolve` races every configured endpoint and takes whichever answers first.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// API key sent as `X-API-Key`. Prefer `api_key_file` to keep the secret out
    /// of a file you might paste or commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Path to a file containing only the key. `~` is expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_file: Option<String>,

    /// Milliseconds to wait for an endpoint's health check before giving up.
    #[serde(default = "default_probe_ms")]
    pub probe_timeout_ms: u64,

    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub name: String,
    pub url: String,
}

fn default_probe_ms() -> u64 {
    // A LAN slskd answers in ~15ms and a proxied one over the internet in
    // ~450ms. The only time this budget is actually spent is when a configured
    // endpoint is unreachable and the connection has to time out, which is
    // exactly the off-network case -- so keep it tight.
    800
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: None,
            api_key_file: Some("~/.config/slskd-cli/.apikey".into()),
            probe_timeout_ms: default_probe_ms(),
            endpoints: vec![Endpoint {
                name: "local".into(),
                url: "http://localhost:5030".into(),
            }],
        }
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let d = directories::BaseDirs::new().ok_or_else(|| anyhow!("no home directory"))?;
    Ok(d.home_dir().join(".config").join("slskd-cli"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(d) = directories::BaseDirs::new() {
            return d.home_dir().join(rest);
        }
    }
    PathBuf::from(p)
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Err(anyhow!(
                "no config at {}\n\nrun `slsk init` to create one",
                path.display()
            ));
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Look up a configured endpoint by name, case-insensitively.
    pub fn endpoint_named(&self, name: &str) -> Result<Endpoint> {
        self.endpoints
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "no endpoint named {name:?}. Configured: {}",
                    self.endpoints
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    /// Decide which endpoint to talk to.
    ///
    /// Precedence, most explicit first:
    ///   1. `--url` on the command line
    ///   2. `SLSKD_URL` in the environment
    ///   3. `--endpoint <name>` on the command line
    ///   4. `SLSKD_ENDPOINT` in the environment
    ///   5. probe every configured endpoint and take the first healthy one
    ///
    /// The environment layers exist so that moving between networks needs no
    /// file edit -- `SLSKD_URL=https://... slsk search ...` is enough, and
    /// exporting it in a shell profile pins a machine to one path.
    pub async fn select(
        &self,
        http: &reqwest::Client,
        url_flag: Option<&str>,
        endpoint_flag: Option<&str>,
    ) -> Result<Endpoint> {
        if let Some(u) = url_flag {
            return Ok(Endpoint { name: "--url".into(), url: u.to_string() });
        }
        if let Ok(u) = std::env::var("SLSKD_URL") {
            if !u.trim().is_empty() {
                return Ok(Endpoint { name: "SLSKD_URL".into(), url: u.trim().to_string() });
            }
        }
        if let Some(n) = endpoint_flag {
            return self.endpoint_named(n);
        }
        if let Ok(n) = std::env::var("SLSKD_ENDPOINT") {
            if !n.trim().is_empty() {
                return self.endpoint_named(n.trim());
            }
        }
        self.resolve(http).await
    }

    /// Resolve the key from `api_key`, then `api_key_file`, then the
    /// `SLSKD_API_KEY` environment variable.
    pub fn key(&self) -> Result<String> {
        if let Ok(k) = std::env::var("SLSKD_API_KEY") {
            if !k.trim().is_empty() {
                return Ok(k.trim().to_string());
            }
        }
        if let Some(k) = &self.api_key {
            if !k.trim().is_empty() {
                return Ok(k.trim().to_string());
            }
        }
        if let Some(f) = &self.api_key_file {
            let path = expand_tilde(f);
            let k = std::fs::read_to_string(&path)
                .with_context(|| format!("reading api key from {}", path.display()))?;
            if !k.trim().is_empty() {
                return Ok(k.trim().to_string());
            }
        }
        Err(anyhow!(
            "no API key found — set `api_key_file` in config.toml or export SLSKD_API_KEY"
        ))
    }

    /// Probe every endpoint concurrently and return the first healthy one.
    ///
    /// Endpoints are ordered by preference in the config; among those that
    /// answer within the timeout, the earliest-listed wins. That keeps a LAN
    /// address ahead of a public hostname without making the user choose.
    pub async fn resolve(&self, http: &reqwest::Client) -> Result<Endpoint> {
        if self.endpoints.is_empty() {
            return Err(anyhow!("no endpoints configured"));
        }
        if self.endpoints.len() == 1 {
            return Ok(self.endpoints[0].clone());
        }

        let timeout = Duration::from_millis(self.probe_timeout_ms);
        let mut set = tokio::task::JoinSet::new();
        for (idx, ep) in self.endpoints.iter().enumerate() {
            let url = format!("{}/health", ep.url.trim_end_matches('/'));
            let http = http.clone();
            set.spawn(async move {
                let ok = http
                    .get(&url)
                    .timeout(timeout)
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                (idx, ok)
            });
        }

        let mut best: Option<usize> = None;
        while let Some(res) = set.join_next().await {
            if let Ok((idx, true)) = res {
                best = Some(best.map_or(idx, |b: usize| b.min(idx)));
                // An endpoint earlier in the list can still win, but if the
                // first one is already healthy nothing can beat it.
                if best == Some(0) {
                    break;
                }
            }
        }

        match best {
            Some(i) => Ok(self.endpoints[i].clone()),
            None => Err(anyhow!(
                "no endpoint responded within {}ms — tried: {}",
                self.probe_timeout_ms,
                self.endpoints
                    .iter()
                    .map(|e| e.url.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Write a starter config, without clobbering an existing one.
    pub fn write_default() -> Result<PathBuf> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = config_path()?;
        if path.exists() {
            return Err(anyhow!("{} already exists", path.display()));
        }
        let sample = r#"# slskd-cli configuration
#
# The API key is read from api_key_file (recommended), the api_key field, or the
# SLSKD_API_KEY environment variable — in that order of preference, reversed:
# the environment wins, then api_key, then the file.
api_key_file = "~/.config/slskd-cli/.apikey"

# How long to wait for an endpoint health check before writing it off. This
# budget is only actually spent when an endpoint is unreachable, which is the
# off-network case, so keep it tight.
probe_timeout_ms = 800

# Endpoints are probed concurrently and the first healthy one wins ties by
# order, so list the fastest path first. A LAN address and a public hostname
# for the same server is the common case.
#
# To override without editing this file:
#   SLSKD_ENDPOINT=remote slsk status     # pick one by name
#   SLSKD_URL=http://host:5030 slsk ...   # bypass config entirely
[[endpoints]]
name = "lan"
url = "http://slskd.local:5030"

[[endpoints]]
name = "remote"
url = "https://slskd.example.com"
"#;
        std::fs::write(&path, sample)?;
        Ok(path)
    }
}
