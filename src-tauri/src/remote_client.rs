//! Remote Mode — 本机 client 模块
//!
//! 负责向 Mini 侧 HTTP API 发起请求。所有函数纯异步，返回 Result。
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::account::Account;

const AUTH_HEADER: &str = "X-Auth-Token";
const DEFAULT_TIMEOUT_SECS: u64 = 10;
const PROBE_TIMEOUT_SECS: u64 = 2;
const CACHE_TTL_SECS: u64 = 60;

struct UrlCache {
    url: String,
    at: Instant,
}

fn url_cache() -> &'static Mutex<Option<UrlCache>> {
    static CACHE: OnceLock<Mutex<Option<UrlCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_url() -> Option<String> {
    let guard = url_cache().lock().ok()?;
    let c = guard.as_ref()?;
    if c.at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
        Some(c.url.clone())
    } else {
        None
    }
}

fn set_cached_url(url: &str) {
    if let Ok(mut g) = url_cache().lock() {
        *g = Some(UrlCache {
            url: url.to_string(),
            at: Instant::now(),
        });
    }
}

pub fn invalidate_cached_url() {
    if let Ok(mut g) = url_cache().lock() {
        *g = None;
    }
}

async fn probe(url: &str) -> bool {
    let Ok(c) = Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build()
    else {
        return false;
    };
    let probe_url = format!("{}/health", trim_url(url));
    c.get(probe_url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// 从 primary 和 fallback 中挑一个可用地址，带 60s 缓存。
pub async fn resolve_base_url(primary: &str, fallback: &str) -> Result<String, String> {
    let p = primary.trim();
    let f = fallback.trim();
    if p.is_empty() && f.is_empty() {
        return Err("未配置 Mini 地址".to_string());
    }
    if let Some(c) = cached_url() {
        if c == p || c == f {
            return Ok(c);
        }
    }
    if !p.is_empty() && probe(p).await {
        set_cached_url(p);
        return Ok(p.to_string());
    }
    if !f.is_empty() && probe(f).await {
        set_cached_url(f);
        return Ok(f.to_string());
    }
    Err(format!(
        "Mini 不可达（primary={}, fallback={}）",
        if p.is_empty() { "-" } else { p },
        if f.is_empty() { "-" } else { f }
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHealth {
    pub mode: String,
    pub version: String,
    pub account_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteToken {
    pub auth_json: Value,
    pub refresh_token: Option<String>,
}

fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("构建 HTTP client 失败: {}", e))
}

fn trim_url(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

/// 健康检查（无需密钥也可拿到 Mini 版本等信息）
pub async fn health(base_url: &str) -> Result<RemoteHealth, String> {
    let url = format!("{}/health", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("连接 Mini 失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Mini /health 返回 {}", resp.status()));
    }
    resp.json::<RemoteHealth>()
        .await
        .map_err(|e| format!("解析 /health 响应失败: {}", e))
}

/// 测试连接+密钥是否正确（会拉 /accounts 看是否 200）
pub async fn test_auth(base_url: &str, secret: &str) -> Result<RemoteHealth, String> {
    let h = health(base_url).await?;
    let url = format!("{}/accounts", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("连接失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Mini /accounts 返回 {}", resp.status()));
    }
    Ok(h)
}

/// 列出 Mini 上的所有账号
pub async fn list_accounts(base_url: &str, secret: &str) -> Result<Vec<Account>, String> {
    let url = format!("{}/accounts", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Mini 返回 {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = body
        .get("accounts")
        .ok_or("响应缺少 accounts 字段")?
        .clone();
    serde_json::from_value(arr).map_err(|e| format!("反序列化账号列表失败: {}", e))
}

/// 上传（upsert）单个账号到 Mini
pub async fn upsert_account(
    base_url: &str,
    secret: &str,
    account: &Account,
) -> Result<(), String> {
    let url = format!("{}/accounts", trim_url(base_url));
    let payload = serde_json::json!({ "account": account });
    let resp = client()?
        .post(&url)
        .header(AUTH_HEADER, secret)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("POST 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Mini 返回 {}: {}", status, body));
    }
    Ok(())
}

/// 删除 Mini 上指定账号
pub async fn delete_account(base_url: &str, secret: &str, id: &str) -> Result<(), String> {
    let url = format!("{}/accounts/{}", trim_url(base_url), id);
    let resp = client()?
        .delete(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("DELETE 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(format!("Mini 返回 {}", resp.status()));
    }
    Ok(())
}

/// 拉取指定账号最新的 auth_json（Mini 侧保活已保证新鲜）
pub async fn fetch_token(base_url: &str, secret: &str, id: &str) -> Result<RemoteToken, String> {
    let url = format!("{}/accounts/{}/token", trim_url(base_url), id);
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("GET token 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Mini 返回 {}", resp.status()));
    }
    resp.json::<RemoteToken>()
        .await
        .map_err(|e| format!("解析 token 响应失败: {}", e))
}
