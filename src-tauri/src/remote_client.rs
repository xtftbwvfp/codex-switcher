//! Remote Mode — 本机 client 模块
//!
//! 负责向 Server 侧 HTTP API 发起请求。所有函数纯异步，返回 Result。
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
        return Err("未配置 Server 地址".to_string());
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
        "Server 不可达（primary={}, fallback={}）",
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

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteQuotaEntry {
    pub id: String,
    #[serde(default)]
    pub cached_quota: Option<crate::account::CachedQuota>,
    #[serde(default)]
    pub is_banned: bool,
    #[serde(default)]
    pub is_token_invalid: bool,
    #[serde(default)]
    pub is_logged_out: bool,
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

/// 健康检查（无需密钥也可拿到 Server 版本等信息）
pub async fn health(base_url: &str) -> Result<RemoteHealth, String> {
    let url = format!("{}/health", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("连接 Server 失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Server /health 返回 {}", resp.status()));
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
        return Err(format!("Server /accounts 返回 {}", resp.status()));
    }
    Ok(h)
}

/// 列出 Server 上的所有账号
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
        return Err(format!("Server 返回 {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = body
        .get("accounts")
        .ok_or("响应缺少 accounts 字段")?
        .clone();
    serde_json::from_value(arr).map_err(|e| format!("反序列化账号列表失败: {}", e))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertOutcome {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub upserted: String,
    #[serde(default)]
    pub quota_refreshed: bool,
    #[serde(default)]
    pub quota_error: Option<String>,
}

/// 上传（upsert）单个账号到 Server。Server 会在写入后尝试刷新一次额度。
pub async fn upsert_account(
    base_url: &str,
    secret: &str,
    account: &Account,
) -> Result<UpsertOutcome, String> {
    let url = format!("{}/accounts", trim_url(base_url));
    let payload = serde_json::json!({ "account": account });
    // 上传 + 服务端刷额度 可能 10+ 秒，临时给 45s 超时
    let c = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = c
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
        return Err(format!("Server 返回 {}: {}", status, body));
    }
    resp.json::<UpsertOutcome>()
        .await
        .map_err(|e| format!("解析 upsert 响应失败: {}", e))
}

/// 删除 Server 上指定账号
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
        return Err(format!("Server 返回 {}", resp.status()));
    }
    Ok(())
}

/// 拉取 Server 上所有账号的配额数据（client 模式下替代本地 quota_refresh）
pub async fn fetch_all_quota(
    base_url: &str,
    secret: &str,
) -> Result<Vec<RemoteQuotaEntry>, String> {
    let url = format!("{}/quotas", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("GET /quotas 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = body.get("quotas").ok_or("响应缺少 quotas 字段")?.clone();
    serde_json::from_value(arr).map_err(|e| format!("反序列化 quotas 失败: {}", e))
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteCurrent {
    pub current: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cached_quota: Option<crate::account::CachedQuota>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteSwitchOutcome {
    #[serde(default)]
    pub switched: bool,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub exhausted: bool,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub earliest_reset_at: Option<i64>,
}

pub async fn get_current(base_url: &str, secret: &str) -> Result<RemoteCurrent, String> {
    let url = format!("{}/current", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("GET /current 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    resp.json::<RemoteCurrent>()
        .await
        .map_err(|e| format!("解析 /current 响应失败: {}", e))
}

pub async fn request_switch(
    base_url: &str,
    secret: &str,
    from: Option<&str>,
    reason: &str,
) -> Result<RemoteSwitchOutcome, String> {
    let url = format!("{}/switch", trim_url(base_url));
    let body = serde_json::json!({ "from": from, "reason": reason });
    let resp = client()?
        .post(&url)
        .header(AUTH_HEADER, secret)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&body).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("POST /switch 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    resp.json::<RemoteSwitchOutcome>()
        .await
        .map_err(|e| format!("解析 /switch 响应失败: {}", e))
}

/// 列出 Server 已安装的 skill 目录名
pub async fn list_remote_skills(base_url: &str, secret: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/skills", trim_url(base_url));
    let resp = client()?
        .get(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("GET /skills 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = body.get("skills").ok_or("响应缺少 skills 字段")?.clone();
    serde_json::from_value(arr).map_err(|e| format!("反序列化 skills 失败: {}", e))
}

/// 将一个 skill zip 推送到 Server
pub async fn upload_skill(
    base_url: &str,
    secret: &str,
    name: &str,
    zip_bytes: Vec<u8>,
) -> Result<(), String> {
    let url = format!(
        "{}/skills/upload?name={}",
        trim_url(base_url),
        url_encode(name)
    );
    let resp = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("构建 HTTP client 失败: {}", e))?
        .post(&url)
        .header(AUTH_HEADER, secret)
        .header("Content-Type", "application/zip")
        .body(zip_bytes)
        .send()
        .await
        .map_err(|e| format!("POST /skills/upload 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server 返回 {}: {}", status, body));
    }
    Ok(())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 请 Server 对某账号执行一次额度刷新，返回最新 UsageDisplay。
/// client 模式下本机不持 token，刷新必须由 Server 完成。
pub async fn refresh_account_quota(
    base_url: &str,
    secret: &str,
    id: &str,
) -> Result<crate::usage::UsageDisplay, String> {
    let url = format!("{}/accounts/{}/refresh", trim_url(base_url), id);
    // oauth refresh + /usage 可能 15s+，给 45s 超时
    let c = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = c
        .post(&url)
        .header(AUTH_HEADER, secret)
        .send()
        .await
        .map_err(|e| format!("POST /accounts/{}/refresh 失败: {}", id, e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析刷新响应失败: {}", e))?;
    if !status.is_success() {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误")
            .to_string();
        return Err(err);
    }
    let usage = body
        .get("usage")
        .cloned()
        .ok_or("响应缺少 usage 字段")?;
    serde_json::from_value(usage).map_err(|e| format!("反序列化 usage 失败: {}", e))
}

/// solo 模式心跳：通知 Server "本机在接管全部保活"。
/// Server 收到后 TTL 内会跳过自己的 quota_refresh 循环，避免双端同时 refresh 撞 rotate。
pub async fn send_solo_heartbeat(
    base_url: &str,
    secret: &str,
    ttl_secs: i64,
) -> Result<(), String> {
    let url = format!("{}/solo/heartbeat", trim_url(base_url));
    let body = serde_json::json!({ "ttl_secs": ttl_secs });
    let resp = client()?
        .post(&url)
        .header(AUTH_HEADER, secret)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&body).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("心跳失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    Ok(())
}

/// solo 模式切号同步：告诉 Server 本机刚切到了 new_id。Server 仅记录 current，不重跑选号。
pub async fn push_solo_switch(
    base_url: &str,
    secret: &str,
    new_id: &str,
) -> Result<(), String> {
    let url = format!("{}/solo/current", trim_url(base_url));
    let body = serde_json::json!({ "current": new_id });
    let resp = client()?
        .post(&url)
        .header(AUTH_HEADER, secret)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&body).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("push /solo/current 失败: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("共享密钥不正确".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Server 返回 {}", resp.status()));
    }
    Ok(())
}

/// 拉取指定账号最新的 auth_json（Server 侧保活已保证新鲜）
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
        return Err(format!("Server 返回 {}", resp.status()));
    }
    resp.json::<RemoteToken>()
        .await
        .map_err(|e| format!("解析 token 响应失败: {}", e))
}
