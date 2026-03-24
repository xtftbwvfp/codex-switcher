//! Codex Switcher - 本地 HTTP/WebSocket 代理服务器
//!
//! 透明代理：拦截 Codex CLI/App 请求，动态注入当前账号 Token 并转发。
//! HTTP: Header 转发逻辑与官方 responses-api-proxy 一致
//! WebSocket: 双向桥接，支持 Codex App 的 WebSocket 通信
//!
//! 功能：SSE 流式转发 | WebSocket 透传 | 429 自动切号 | 封号检测 | 评分选号

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
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
use tokio_tungstenite::tungstenite;
use tungstenite::client::IntoClientRequest;

use crate::account::AccountStore;
use crate::token_tracker::TokenTracker;

/// ChatGPT OAuth 登录用的上游（免费/Plus/Team 账号）
const CHATGPT_HOST: &str = "chatgpt.com";
const CHATGPT_ORIGIN: &str = "https://chatgpt.com/backend-api/codex";

/// API key 用的上游
const API_HOST: &str = "api.openai.com";
const API_ORIGIN: &str = "https://api.openai.com";
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
    /// 切号时通知 WebSocket 断开
    ws_disconnect: Arc<tokio::sync::Notify>,
}

/// 启动代理服务器
pub fn start(
    store: Arc<Mutex<AccountStore>>,
    port: u16,
    app_handle: tauri::AppHandle,
    stats: Arc<ProxyStats>,
    tracker: Arc<TokenTracker>,
    ws_disconnect: Arc<tokio::sync::Notify>,
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
            ws_disconnect,
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
                    .with_upgrades()
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

/// 获取当前账号最新的 access_token + 认证模式
///
/// 安全策略：不主动刷新 token（避免与 Codex CLI 冲突），
/// 但每次请求从 auth.json 回读，确保用到 Codex CLI 刷新后的最新值。
///
/// 返回 (token, is_chatgpt_auth)
fn get_current_token(state: &ProxyState) -> Result<(String, bool), String> {
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

    let token = AccountStore::extract_access_token(&account.auth_json)
        .ok_or_else(|| "当前账号缺少 access_token".to_string())?;

    // 判断认证模式：JWT (eyJ...) = ChatGPT OAuth, sk-... = API key
    let is_chatgpt = token.starts_with("eyJ");

    Ok((token, is_chatgpt))
}

/// 根据认证模式获取上游地址
fn get_upstream(is_chatgpt: bool, path_and_query: &str) -> (String, &'static str) {
    if is_chatgpt {
        // 客户端路径: /v1/responses (因为 OPENAI_BASE_URL 带 /v1)
        // ChatGPT 上游: /backend-api/codex/responses (不含 /v1)
        // 需要去掉 /v1 前缀
        let path = path_and_query
            .strip_prefix("/v1")
            .unwrap_or(path_and_query);
        let url = format!("{}{}", CHATGPT_ORIGIN, path);
        (url, CHATGPT_HOST)
    } else {
        // API key: 转发到 api.openai.com + 原始路径（保留 /v1）
        let url = format!("{}{}", API_ORIGIN, path_and_query);
        (url, API_HOST)
    }
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
    state.ws_disconnect.notify_waiters(); // 断开 WebSocket 让 Codex App 重连
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

    // ── WebSocket 升级检测 ──
    if is_websocket_upgrade(&req) {
        println!("[Proxy] WebSocket upgrade 请求: {}", req.uri());
        return handle_websocket(state, req).await;
    }

    // 1. 获取当前 token + 认证模式
    let (token, is_chatgpt) = match get_current_token(&state) {
        Ok(t) => t,
        Err(e) => return Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, &e)),
    };

    // 2. 提取请求元数据 + 根据认证模式路由上游
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let (upstream_url, upstream_host) = get_upstream(is_chatgpt, &path_and_query);

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
    if let Ok(host_val) = reqwest::header::HeaderValue::from_str(upstream_host) {
        base_headers.insert(reqwest::header::HOST, host_val);
    }

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
            if let Ok((new_token, _)) = get_current_token(&state) {
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

    // 流式传输 + usage 提取
    // 每个 chunk 直接转发，同时复制到 buffer
    // 流结束时（收到 None）解析 buffer 提取 usage
    let usage_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buf_clone = usage_buf.clone();
    let tracker_clone = tracker.clone();

    let raw_stream = upstream_resp.bytes_stream();

    // 用 chain 在原始 stream 结束后追加一个"结束信号"
    // 利用 map + 闭包在最后一个 chunk 后触发 usage 解析
    let stream = raw_stream.map(move |result| {
        match result {
            Ok(bytes) => {
                if let Ok(mut buf) = buf_clone.lock() {
                    buf.extend_from_slice(&bytes);
                }
                Ok(Frame::data(bytes))
            }
            Err(e) => Err(e.to_string()),
        }
    });

    // 用 chain + once 在流结束后触发解析
    let buf_for_end = usage_buf;
    let end_signal = futures_util::stream::once(async move {
        // 流结束，解析 buffer
        if let Some(tracker) = tracker_clone {
            if let Ok(buf) = buf_for_end.lock() {
                if !buf.is_empty() {
                    if let Some(usage) =
                        crate::token_tracker::extract_usage_from_sse(&buf, "")
                    {
                        println!(
                            "[Proxy] Token 统计: input={} output={} total={} model={}",
                            usage.input_tokens, usage.output_tokens,
                            usage.total_tokens, usage.model
                        );
                        tracker.record(usage);
                    }
                }
            }
        }
        // 不产生数据帧，只是触发解析
        Err("".to_string()) // 这个 Err 会被 StreamBody 忽略
    })
    // 过滤掉这个空 error，不让它传到客户端
    .filter(|_| futures_util::future::ready(false));

    let combined = stream.chain(end_signal);

    builder
        .body(BodyExt::boxed(StreamBody::new(combined)))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "流构建失败"))
}

/// Full body 包装（用于错误响应等小数据）
fn full_body(bytes: Bytes) -> ProxyBody {
    Full::new(bytes).map_err(|_| String::new()).boxed()
}

// ────────────────────────────────────────────────────────────────
// WebSocket 代理
// ────────────────────────────────────────────────────────────────

/// 检测是否为 WebSocket 升级请求
fn is_websocket_upgrade(req: &Request<Incoming>) -> bool {
    req.headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase().contains("websocket"))
        .unwrap_or(false)
}

/// 处理 WebSocket 代理：连接上游 + 双向桥接
async fn handle_websocket(
    state: Arc<ProxyState>,
    mut req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    // 1. 获取 token 和上游地址
    let (mut token, mut is_chatgpt) = match get_current_token(&state) {
        Ok(t) => t,
        Err(e) => return Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, &e)),
    };

    // 预检：如果当前账号没额度，先切号再连接
    {
        let should_switch = {
            let store = match state.store.lock() {
                Ok(s) => s,
                Err(_) => return Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, "锁失败")),
            };
            if let Some(current_id) = &store.current {
                store.accounts.get(current_id).and_then(|a| {
                    a.cached_quota.as_ref().map(|q| {
                        let is_free = q.plan_type.to_lowercase() == "free";
                        if is_free {
                            q.five_hour_left <= 0.0
                        } else {
                            q.five_hour_left <= 0.0 || q.weekly_left <= 0.0
                        }
                    })
                }).unwrap_or(false)
            } else {
                false
            }
        };

        if should_switch {
            println!("[Proxy] WebSocket 预检：当前账号无额度，尝试切号...");
            if let PickResult::Found { id, token: new_token } = pick_next_account(&state) {
                if do_switch(&state, &id).is_ok() {
                    is_chatgpt = new_token.starts_with("eyJ");
                    token = new_token;
                    println!("[Proxy] WebSocket 预检切号成功");
                }
            }
        }
    }

    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let (http_url, _upstream_host) = get_upstream(is_chatgpt, &path);

    // http(s):// → ws(s)://
    let ws_url = http_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);

    // 2. 构建上游 WebSocket 请求（透明 header 转发 + token 注入）
    let mut upstream_req: tungstenite::http::Request<()> = match ws_url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => {
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                &format!("WebSocket 请求构建失败: {}", e),
            ))
        }
    };

    // 转发客户端 header（排除 WebSocket 握手专用 header，由 into_client_request 生成）
    for (name, value) in req.headers() {
        let lower = name.as_str().to_lowercase();
        if matches!(
            lower.as_str(),
            "authorization"
                | "host"
                | "upgrade"
                | "connection"
                | "sec-websocket-key"
                | "sec-websocket-version"
                | "sec-websocket-extensions"
        ) {
            continue;
        }
        upstream_req.headers_mut().insert(name.clone(), value.clone());
    }

    // 注入 token
    if let Ok(auth_val) = HeaderValue::from_str(&format!("Bearer {}", token)) {
        upstream_req
            .headers_mut()
            .insert(hyper::header::AUTHORIZATION, auth_val);
    }

    // 3. 连接上游 WebSocket
    let (upstream_ws, upstream_handshake_resp) =
        match tokio_tungstenite::connect_async(upstream_req).await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("[Proxy] WebSocket 上游连接失败: {}", e);
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("WebSocket 上游连接失败: {}", e),
                ));
            }
        };

    println!("[Proxy] WebSocket 上游已连接");

    // 4. 计算 Sec-WebSocket-Accept 回复客户端
    let ws_key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let accept_key = tungstenite::handshake::derive_accept_key(ws_key.as_bytes());

    // 5. 提取 hyper upgrade handle（必须在返回 101 之前）
    let on_upgrade = hyper::upgrade::on(&mut req);

    // 6. 构建 101 响应，转发上游的响应 header（x-codex-turn-state 等）
    let mut response_builder = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header("Upgrade", "websocket")
        .header("Connection", "Upgrade")
        .header("Sec-WebSocket-Accept", &accept_key);

    // 转发上游响应 header（排除 WebSocket 握手 header）
    for (name, value) in upstream_handshake_resp.headers() {
        let lower = name.as_str().to_lowercase();
        if matches!(
            lower.as_str(),
            "upgrade" | "connection" | "sec-websocket-accept" | "sec-websocket-extensions"
                | "content-length" | "transfer-encoding"
        ) {
            continue;
        }
        if let Ok(hn) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            response_builder = response_builder.header(hn, value.clone());
        }
    }

    let response = response_builder
        .body(full_body(Bytes::new()))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "101 构建失败"));

    // 7. 后台任务：upgrade 完成后双向桥接
    let disconnect = state.ws_disconnect.clone();
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let client_ws =
                    tokio_tungstenite::WebSocketStream::from_raw_socket(
                        io,
                        tungstenite::protocol::Role::Server,
                        None,
                    )
                    .await;

                println!("[Proxy] WebSocket 客户端已升级，开始桥接");
                bridge_websockets(client_ws, upstream_ws, disconnect, state).await;
                println!("[Proxy] WebSocket 连接已关闭");
            }
            Err(e) => eprintln!("[Proxy] WebSocket upgrade 失败: {}", e),
        }
    });

    Ok(response)
}

/// 检测 WebSocket 消息是否为限额错误
/// 只匹配 response.failed 类型的错误消息，避免误判正常消息中的 rate_limit 字段
fn detect_ws_rate_limit(msg: &tungstenite::Message) -> bool {
    if let tungstenite::Message::Text(ref text) = msg {
        // 必须是错误类型的消息
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            // 只在 response.failed 或 error 类型中检测
            if msg_type == "response.failed" || msg_type == "error" {
                let error_code = val
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let error_msg = val
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                return error_code == "rate_limit_exceeded"
                    || error_msg.contains("hit your usage limit")
                    || error_msg.contains("usage limit");
            }
        }
    }
    false
}

/// 检测 WebSocket 消息是否为封号错误
fn detect_ws_banned(msg: &tungstenite::Message) -> bool {
    if let tungstenite::Message::Text(ref text) = msg {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type == "response.failed" || msg_type == "error" {
                let error_msg = val
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                return error_msg.contains("deactivated")
                    || error_msg.contains("banned")
                    || error_msg.contains("suspended");
            }
        }
    }
    false
}

/// 双向桥接两个 WebSocket 连接
/// - 切号信号 → 断开连接
/// - 检测到限额/封号消息 → 断开连接（代理会在下次连接时预检切号）
async fn bridge_websockets<S1, S2>(
    client: S1,
    upstream: S2,
    disconnect: Arc<tokio::sync::Notify>,
    state: Arc<ProxyState>,
)
where
    S1: futures_util::Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
        + futures_util::Sink<tungstenite::Message, Error = tungstenite::Error>
        + Unpin,
    S2: futures_util::Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
        + futures_util::Sink<tungstenite::Message, Error = tungstenite::Error>
        + Unpin,
{
    let (mut client_write, mut client_read) = client.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();

    let client_to_upstream = async {
        while let Some(msg) = client_read.next().await {
            match msg {
                Ok(msg) => {
                    if msg.is_close() {
                        let _ = upstream_write.send(msg).await;
                        break;
                    }
                    if upstream_write.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    };

    let state_clone = state.clone();
    let upstream_to_client = async {
        while let Some(msg) = upstream_read.next().await {
            match msg {
                Ok(msg) => {
                    // 检测限额错误（仅解析 response.failed 类型）
                    if detect_ws_rate_limit(&msg) {
                        println!("[Proxy] WebSocket 检测到限额错误（response.failed），触发切号...");
                        mark_current_quota_depleted(&state_clone);
                        let _ = client_write.send(msg).await;
                        if let PickResult::Found { id, .. } = pick_next_account(&state_clone) {
                            let _ = do_switch(&state_clone, &id);
                        }
                        break;
                    }
                    // 检测封号
                    if detect_ws_banned(&msg) {
                        println!("[Proxy] WebSocket 检测到封号（response.failed），触发切号...");
                        mark_current_banned(&state_clone);
                        let _ = client_write.send(msg).await;
                        if let PickResult::Found { id, .. } = pick_next_account(&state_clone) {
                            let _ = do_switch(&state_clone, &id);
                        }
                        break;
                    }

                    if msg.is_close() {
                        let _ = client_write.send(msg).await;
                        break;
                    }
                    if client_write.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    };

    tokio::select! {
        _ = client_to_upstream => {},
        _ = upstream_to_client => {},
        _ = disconnect.notified() => {
            println!("[Proxy] 账号已切换，断开 WebSocket 连接（Codex App 将自动重连）");
        },
    }
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
