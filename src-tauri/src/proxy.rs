//! Codex Switcher - 本地 HTTP 代理服务器
//!
//! 透明代理：拦截 Codex CLI 请求，动态注入当前账号 Token 并转发到 api.openai.com。
//! Header 转发逻辑与官方 codex-rs/responses-api-proxy 完全一致，确保请求指纹对齐。
//!
//! 功能：SSE 流式转发 | 429 自动切号 | 封号检测 | 预防性阈值 | 评分选号 | auth.json 回读

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use tauri::Emitter;
use tokio::net::TcpListener;

use crate::account::AccountStore;
use crate::token_tracker::TokenTracker;

const UPSTREAM_HOST: &str = "api.openai.com";
const UPSTREAM_ORIGIN: &str = "https://api.openai.com";
const MAX_429_RETRIES: usize = 2;

/// 统一的响应 Body 类型：支持 Full（错误/小响应）和 Stream（SSE 流式）
type ProxyBody = BoxBody<Bytes, String>;

/// 代理运行指标（与 AppState 共享）
pub struct ProxyStats {
    pub total_requests: AtomicU64,
    pub auto_switches: AtomicU64,
}

impl Default for ProxyStats {
    fn default() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            auto_switches: AtomicU64::new(0),
        }
    }
}

/// 代理运行时共享状态
struct ProxyState {
    store: Arc<Mutex<AccountStore>>,
    client: Client,
    app_handle: tauri::AppHandle,
    switching: AtomicBool,
    stats: Arc<ProxyStats>,
    tracker: Arc<TokenTracker>,
}

/// 启动代理服务器
pub fn start(
    store: Arc<Mutex<AccountStore>>,
    port: u16,
    app_handle: tauri::AppHandle,
    stats: Arc<ProxyStats>,
    tracker: Arc<TokenTracker>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[Proxy] 绑定端口 {} 失败: {}", port, e);
                return;
            }
        };

        println!("[Proxy] 代理服务器已启动，监听 127.0.0.1:{}", port);

        let client = Client::builder()
            .build()
            .expect("[Proxy] 构建 reqwest Client 失败");

        let state = Arc::new(ProxyState {
            store,
            client,
            app_handle,
            switching: AtomicBool::new(false),
            stats,
            tracker,
        });

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[Proxy] accept 失败: {}", e);
                    continue;
                }
            };

            let state = state.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req| {
                    let state = state.clone();
                    handle_request(state, req)
                });

                if let Err(e) = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(io, service)
                    .await
                {
                    if !e.is_incomplete_message() {
                        eprintln!("[Proxy] 连接 {} 错误: {}", peer_addr, e);
                    }
                }
            });
        }
    })
}

// ────────────────────────────────────────────────────────────────
// Token 管理
// ────────────────────────────────────────────────────────────────

/// 获取当前账号最新的 access_token
///
/// 安全策略：不主动刷新 token（避免与 Codex CLI 冲突），
/// 但每次请求从 auth.json 回读，确保用到 Codex CLI 刷新后的最新值。
fn get_current_token(state: &ProxyState) -> Result<String, String> {
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    let current_id = store.current.as_ref().ok_or("没有激活的账号")?.clone();

    // 从 auth.json 回读最新 token（Codex CLI 可能已刷新）
    if let Ok(disk_auth) = AccountStore::read_codex_auth() {
        if store.sync_account_from_auth_json(&current_id, disk_auth) {
            let _ = store.save();
        }
    }

    let account = store
        .accounts
        .get(&current_id)
        .ok_or("当前账号不存在")?;

    AccountStore::extract_access_token(&account.auth_json)
        .ok_or_else(|| "当前账号缺少 access_token".to_string())
}

// ────────────────────────────────────────────────────────────────
// 选号算法（复用 lib.rs 共享评分）
// ────────────────────────────────────────────────────────────────

enum PickResult {
    Found { id: String, token: String },
    Exhausted { earliest_reset: Option<i64> },
}

fn pick_next_account(state: &ProxyState) -> PickResult {
    let store = match state.store.lock() {
        Ok(s) => s,
        Err(_) => return PickResult::Exhausted { earliest_reset: None },
    };

    let candidates = crate::score_candidate_accounts(&store);

    if candidates.is_empty() {
        let now = Utc::now().timestamp();
        let mut earliest: Option<i64> = None;
        for account in store.accounts.values() {
            if let Some(q) = &account.cached_quota {
                for r in [q.five_hour_reset_at, q.weekly_reset_at].into_iter().flatten() {
                    if now < r {
                        earliest = Some(earliest.map_or(r, |e: i64| e.min(r)));
                    }
                }
            }
        }
        return PickResult::Exhausted { earliest_reset: earliest };
    }

    let (id, _, _) = &candidates[0];
    if let Some(account) = store.accounts.get(id) {
        if let Some(token) = AccountStore::extract_access_token(&account.auth_json) {
            return PickResult::Found {
                id: id.clone(),
                token,
            };
        }
    }

    PickResult::Exhausted { earliest_reset: None }
}

// ────────────────────────────────────────────────────────────────
// 预防性切号 / 封号检测 / 切号执行
// ────────────────────────────────────────────────────────────────

fn should_preemptive_switch(state: &ProxyState) -> bool {
    let store = match state.store.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };

    let (t5h, tw, fg) = (
        store.settings.proxy_threshold_5h as f64,
        store.settings.proxy_threshold_weekly as f64,
        store.settings.proxy_free_guard as f64,
    );

    if t5h == 0.0 && tw == 0.0 && fg == 0.0 {
        return false;
    }

    let current_id = match &store.current {
        Some(id) => id,
        None => return false,
    };
    let quota = match store.accounts.get(current_id).and_then(|a| a.cached_quota.as_ref()) {
        Some(q) => q,
        None => return false,
    };

    let plan = quota.plan_type.to_lowercase();
    let is_free = plan == "free" || plan == "unknown";

    if is_free && fg > 0.0 && quota.five_hour_left < fg {
        println!("[Proxy] Free 保护线触发: {:.0}% < {:.0}%", quota.five_hour_left, fg);
        return true;
    }
    if t5h > 0.0 && quota.five_hour_left < t5h {
        println!("[Proxy] 5h 阈值触发: {:.0}% < {:.0}%", quota.five_hour_left, t5h);
        return true;
    }
    if tw > 0.0 && quota.weekly_left < tw {
        println!("[Proxy] 周阈值触发: {:.0}% < {:.0}%", quota.weekly_left, tw);
        return true;
    }
    false
}

fn mark_current_banned(state: &ProxyState) {
    if let Ok(mut store) = state.store.lock() {
        if let Some(current_id) = store.current.clone() {
            if let Some(account) = store.accounts.get_mut(&current_id) {
                account.is_banned = true;
                let name = account.name.clone();
                let _ = store.save();
                println!("[Proxy] 账号 {} 已标记为封号", name);
                let _ = state.app_handle.emit("proxy-account-banned", &name);
            }
        }
    }
}

/// 429 后标记当前账号的 5h 额度为耗尽
fn mark_current_quota_depleted(state: &ProxyState) {
    if let Ok(mut store) = state.store.lock() {
        if let Some(current_id) = store.current.clone() {
            if let Some(account) = store.accounts.get_mut(&current_id) {
                if let Some(ref mut q) = account.cached_quota {
                    q.five_hour_left = 0.0;
                }
                let _ = store.save();
            }
        }
    }
}

fn do_switch(state: &ProxyState, new_id: &str) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    store.switch_to(new_id)?;
    store.save()?;

    let name = store
        .accounts
        .get(new_id)
        .map(|a| a.name.clone())
        .unwrap_or_default();
    println!("[Proxy] 自动切号 → {}", name);

    state.stats.auto_switches.fetch_add(1, Ordering::Relaxed);
    let _ = state.app_handle.emit("proxy-account-switched", &name);
    let _ = state.app_handle.emit("accounts-updated", ());

    Ok(())
}

// ────────────────────────────────────────────────────────────────
// 核心请求处理
// ────────────────────────────────────────────────────────────────

async fn handle_request(
    state: Arc<ProxyState>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    state.stats.total_requests.fetch_add(1, Ordering::Relaxed);

    // ── 健康检查 ──
    if req.method() == Method::GET && req.uri().path() == "/health" {
        let total = state.stats.total_requests.load(Ordering::Relaxed);
        let switches = state.stats.auto_switches.load(Ordering::Relaxed);
        let body = serde_json::json!({
            "status": "ok",
            "total_requests": total,
            "auto_switches": switches,
        });
        return Ok(Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(full_body(Bytes::from(body.to_string())))
            .unwrap());
    }

    // 1. 获取当前 token（含 auth.json 回读）
    let token = match get_current_token(&state) {
        Ok(t) => t,
        Err(e) => return Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, &e)),
    };

    // 2. 提取请求元数据
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let upstream_url = format!("{}{}", UPSTREAM_ORIGIN, path_and_query);

    // 3. 透明 Header 转发（官方 responses-api-proxy 逻辑）
    let mut base_headers = reqwest::header::HeaderMap::new();
    for (name, value) in req.headers() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "authorization" || lower == "host" {
            continue;
        }
        if let Ok(rn) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(rv) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                base_headers.append(rn, rv);
            }
        }
    }
    base_headers.insert(
        reqwest::header::HOST,
        reqwest::header::HeaderValue::from_static(UPSTREAM_HOST),
    );

    // 4. 读取请求体
    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            eprintln!("[Proxy] 读取请求体失败: {}", e);
            return Ok(error_response(StatusCode::BAD_REQUEST, "读取请求体失败"));
        }
    };

    // 5. 首次转发
    let upstream_resp = match forward_with_token(
        &state, &method, &upstream_url, &base_headers, &body_bytes, &token,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                &format!("上游连接失败: {}", e),
            ))
        }
    };

    let status_code = upstream_resp.status();

    // 6. 封号检测（401/403）
    if status_code == reqwest::StatusCode::UNAUTHORIZED
        || status_code == reqwest::StatusCode::FORBIDDEN
    {
        let resp_bytes = upstream_resp.bytes().await.unwrap_or_default();
        let body_lower = String::from_utf8_lossy(&resp_bytes).to_lowercase();
        let banned = body_lower.contains("deactivated")
            || body_lower.contains("banned")
            || body_lower.contains("suspended")
            || body_lower.contains("account_deactivated");

        if banned {
            println!("[Proxy] 封号检测触发，标记并切号...");
            mark_current_banned(&state);

            if let Some(resp) = try_switch_and_retry(
                &state, &method, &upstream_url, &base_headers, &body_bytes,
            )
            .await
            {
                return Ok(resp);
            }
        }

        return Ok(Response::builder()
            .status(status_code.as_u16())
            .header("content-type", "application/json")
            .body(full_body(resp_bytes))
            .unwrap_or_else(|_| error_response(StatusCode::BAD_GATEWAY, "响应构建失败")));
    }

    // 7. 429 自动切号
    if status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
        println!("[Proxy] 收到 429，标记额度耗尽并切号...");
        mark_current_quota_depleted(&state);

        // 并发保护
        if state
            .switching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let result = try_switch_and_retry(
                &state, &method, &upstream_url, &base_headers, &body_bytes,
            )
            .await;
            state.switching.store(false, Ordering::SeqCst);

            if let Some(resp) = result {
                return Ok(resp);
            }
        } else {
            // 其他请求正在切号，短暂等待后用新 token 重试
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Ok(new_token) = get_current_token(&state) {
                if let Ok(retry_resp) = forward_with_token(
                    &state, &method, &upstream_url, &base_headers, &body_bytes, &new_token,
                )
                .await
                {
                    return Ok(build_stream_response(retry_resp, Some(state.tracker.clone())));
                }
            }
        }

        // 切号失败/账号耗尽 → 缓冲原始 429 返回
        let resp_bytes = upstream_resp.bytes().await.unwrap_or_default();
        return Ok(Response::builder()
            .status(429)
            .header("content-type", "application/json")
            .body(full_body(resp_bytes))
            .unwrap_or_else(|_| error_response(StatusCode::TOO_MANY_REQUESTS, "429")));
    }

    // 8. 成功响应 → SSE 流式转发
    let resp = build_stream_response(upstream_resp, Some(state.tracker.clone()));

    // 后台检查预防性切号
    let state_clone = state.clone();
    tokio::spawn(async move {
        if should_preemptive_switch(&state_clone) {
            if state_clone
                .switching
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                if let PickResult::Found { id, .. } = pick_next_account(&state_clone) {
                    let _ = do_switch(&state_clone, &id);
                }
                state_clone.switching.store(false, Ordering::SeqCst);
            }
        }
    });

    Ok(resp)
}

/// 切号并重试（最多 MAX_429_RETRIES 次）
async fn try_switch_and_retry(
    state: &ProxyState,
    method: &hyper::Method,
    upstream_url: &str,
    base_headers: &reqwest::header::HeaderMap,
    body: &Bytes,
) -> Option<Response<ProxyBody>> {
    for attempt in 0..MAX_429_RETRIES {
        match pick_next_account(state) {
            PickResult::Found { id, token } => {
                if let Err(e) = do_switch(state, &id) {
                    eprintln!("[Proxy] 切号失败: {}", e);
                    continue;
                }

                match forward_with_token(state, method, upstream_url, base_headers, body, &token)
                    .await
                {
                    Ok(resp) if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS => {
                        println!(
                            "[Proxy] 第 {} 次切号重试成功 ({})",
                            attempt + 1,
                            resp.status()
                        );
                        return Some(build_stream_response(resp, Some(state.tracker.clone())));
                    }
                    Ok(_) => {
                        println!("[Proxy] 第 {} 次切号后仍 429", attempt + 1);
                        mark_current_quota_depleted(state);
                        continue;
                    }
                    Err(e) => {
                        eprintln!("[Proxy] 切号后转发失败: {}", e);
                        continue;
                    }
                }
            }
            PickResult::Exhausted { earliest_reset } => {
                let msg = if let Some(ts) = earliest_reset {
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
                        .unwrap_or_else(|| "未知".to_string());
                    format!("所有账号额度已耗尽，最早恢复：{}", dt)
                } else {
                    "所有账号额度已耗尽".to_string()
                };
                eprintln!("[Proxy] {}", msg);
                let _ = state.app_handle.emit("proxy-all-exhausted", &msg);
                return None;
            }
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────
// HTTP 转发与响应构建
// ────────────────────────────────────────────────────────────────

async fn forward_with_token(
    state: &ProxyState,
    method: &hyper::Method,
    upstream_url: &str,
    base_headers: &reqwest::header::HeaderMap,
    body: &Bytes,
    token: &str,
) -> Result<reqwest::Response, String> {
    let mut headers = base_headers.clone();
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
        headers.insert(reqwest::header::AUTHORIZATION, v);
    }

    state
        .client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes())
                .unwrap_or(reqwest::Method::POST),
            upstream_url,
        )
        .headers(headers)
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| format!("转发请求失败: {}", e))
}

/// SSE 流式响应构建：复制 header + 流式传输 body + 后台提取 usage
fn build_stream_response(
    upstream_resp: reqwest::Response,
    tracker: Option<Arc<TokenTracker>>,
) -> Response<ProxyBody> {
    let status = upstream_resp.status();
    let mut builder = Response::builder().status(status.as_u16());

    // 尝试从请求中获取 model 信息（响应 header 中可能没有）
    let model_hint = String::new();

    for (name, value) in upstream_resp.headers() {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "trailer" | "upgrade"
        ) {
            continue;
        }
        if let Ok(hn) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(hv) = HeaderValue::from_bytes(value.as_bytes()) {
                builder = builder.header(hn, hv);
            }
        }
    }

    // 流式传输 + 后台 usage 提取
    // 每个 chunk 直接转发（零延迟），同时复制到 buffer 用于解析 usage
    let usage_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buf_clone = usage_buf.clone();

    let raw_stream = upstream_resp.bytes_stream();

    let stream = raw_stream.map(move |result| {
        match result {
            Ok(bytes) => {
                // 复制到 usage buffer（不阻塞转发）
                if let Ok(mut buf) = buf_clone.lock() {
                    buf.extend_from_slice(&bytes);
                }
                Ok(Frame::data(bytes))
            }
            Err(e) => Err(e.to_string()),
        }
    });

    // 当 stream 结束后，后台解析 usage
    if let Some(tracker) = tracker {
        let buf_for_parse = usage_buf;
        let model = model_hint;
        tokio::spawn(async move {
            // 等一小段时间确保 stream 结束
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Ok(buf) = buf_for_parse.lock() {
                if let Some(usage) =
                    crate::token_tracker::extract_usage_from_sse(&buf, &model)
                {
                    tracker.record(usage);
                }
            }
        });
    }

    builder
        .body(BodyExt::boxed(StreamBody::new(stream)))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "流构建失败"))
}

/// Full body 包装（用于错误响应等小数据）
fn full_body(bytes: Bytes) -> ProxyBody {
    Full::new(bytes).map_err(|_| String::new()).boxed()
}

fn error_response(status: StatusCode, message: &str) -> Response<ProxyBody> {
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": "proxy_error",
            "code": status.as_u16(),
        }
    });

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(full_body(Bytes::from(body.to_string())))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full_body(Bytes::from("internal error")))
                .unwrap()
        })
}
