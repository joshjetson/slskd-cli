//! Thin async wrapper over the slskd v0 HTTP API.

use crate::models::*;
use anyhow::{anyhow, Context, Result};
use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    key: String,
}

/// Percent-encode a path segment.
///
/// Soulseek usernames are free-form and routinely contain spaces, `#`, `?` and
/// other characters that are not legal in a URL path. Interpolating one raw
/// produces a request that either fails outright or silently addresses the
/// wrong user, so every username crossing into a URL goes through here.
fn encode_segment(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

impl Client {
    pub fn new(http: reqwest::Client, base: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
            key: key.into(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v0{}", self.base, path)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path);
        let res = self
            .http
            .get(&url)
            .header("X-API-Key", &self.key)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        self.decode(res, &url).await
    }

    async fn post<B: serde::Serialize, T: serde::de::DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.url(path);
        let res = self
            .http
            .post(&url)
            .header("X-API-Key", &self.key)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        self.decode(res, &url).await
    }

    async fn decode<T: serde::de::DeserializeOwned>(
        &self,
        res: reqwest::Response,
        url: &str,
    ) -> Result<T> {
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "401 from slskd — the API key was rejected.\n\
                 Check `web.authentication.api_keys` in slskd.yml and that the key\n\
                 has a role of `readwrite` (readonly cannot queue downloads)."
            ));
        }
        let body = res.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("{status} from {url}: {}", body.trim()));
        }
        if body.trim().is_empty() {
            // 204 and friends; callers use () or a Default type here.
            return serde_json::from_str("null")
                .or_else(|_| serde_json::from_str("{}"))
                .map_err(|e| anyhow!("empty body from {url}: {e}"));
        }
        serde_json::from_str(&body)
            .with_context(|| format!("decoding response from {url}"))
    }

    // ------------------------------------------------------------ endpoints

    pub async fn application(&self) -> Result<Application> {
        self.get("/application").await
    }

    pub async fn start_search(&self, text: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let _: serde_json::Value = self
            .post(
                "/searches",
                &SearchRequest {
                    id: id.clone(),
                    search_text: text.to_string(),
                },
            )
            .await?;
        Ok(id)
    }

    pub async fn search_state(&self, id: &str) -> Result<Search> {
        self.get(&format!("/searches/{id}")).await
    }

    pub async fn search_results(&self, id: &str) -> Result<Search> {
        self.get(&format!("/searches/{id}?includeResponses=true"))
            .await
    }

    pub async fn delete_search(&self, id: &str) -> Result<()> {
        let url = self.url(&format!("/searches/{id}"));
        self.http
            .delete(&url)
            .header("X-API-Key", &self.key)
            .send()
            .await?;
        Ok(())
    }

    /// Run a search to completion, polling until slskd reports a terminal state.
    ///
    /// `"Completed, TimedOut"` is a terminal state and usually the one you get —
    /// a Soulseek search ends when its clock runs out, not when the network is
    /// exhausted. Waiting for anything tidier means waiting forever.
    pub async fn search(&self, text: &str, max_wait: Duration) -> Result<Search> {
        let id = self.start_search(text).await?;
        let deadline = std::time::Instant::now() + max_wait;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let s = self.search_state(&id).await?;
            if !s.in_progress() || std::time::Instant::now() >= deadline {
                break;
            }
        }
        let full = self.search_results(&id).await?;
        let _ = self.delete_search(&id).await;
        Ok(full)
    }

    pub async fn enqueue(&self, username: &str, files: &[File]) -> Result<()> {
        let body: Vec<EnqueueRequest> = files
            .iter()
            .map(|f| EnqueueRequest {
                filename: f.filename.clone(),
                size: f.size,
            })
            .collect();
        let _: serde_json::Value = self
            .post(
                &format!("/transfers/downloads/{}", encode_segment(username)),
                &body,
            )
            .await?;
        Ok(())
    }

    pub async fn downloads(&self) -> Result<Vec<TransferUser>> {
        self.get("/transfers/downloads").await
    }

    pub async fn uploads(&self) -> Result<Vec<TransferUser>> {
        self.get("/transfers/uploads").await
    }

    pub async fn clear_completed(&self) -> Result<()> {
        let url = self.url("/transfers/downloads/all/completed");
        self.http
            .delete(&url)
            .header("X-API-Key", &self.key)
            .send()
            .await?;
        Ok(())
    }

    pub async fn browse(&self, username: &str) -> Result<BrowseResult> {
        self.get(&format!("/users/{}/browse", encode_segment(username)))
            .await
    }

    pub async fn user_info(&self, username: &str) -> Result<UserInfo> {
        self.get(&format!("/users/{}/info", encode_segment(username)))
            .await
    }

    pub async fn user_status(&self, username: &str) -> Result<UserStatus> {
        self.get(&format!("/users/{}/status", encode_segment(username)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::encode_segment;

    #[test]
    fn encodes_characters_that_break_url_paths() {
        // A real username from the network; the space is the common case.
        assert_eq!(encode_segment("Zohran Mamdani"), "Zohran%20Mamdani");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("who?"), "who%3F");
        assert_eq!(encode_segment("tag#1"), "tag%231");
    }

    #[test]
    fn leaves_unreserved_characters_alone() {
        assert_eq!(encode_segment("plain_user-9.x~z"), "plain_user-9.x~z");
    }
}
