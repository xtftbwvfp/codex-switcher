//! Remote Mode — Mini 侧 HTTP API 服务器
//!
//! 提供账号 CRUD 和 token 拉取接口，供本机 client 模式调用。
//! 认证：X-Auth-Token 头必须匹配 settings.remote_shared_secret。
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::account::{Account, AccountStore};

type ResponseBody = Full<Bytes>;

struct ApiState {
    store: Arc<Mutex<AccountStore>>,
    secret: String,
    version: String,
}

pub fn spawn_remote_server(
    store: Arc<Mutex<AccountStore>>,
    bind: String,
    port: u16,
    secret: String,
    version: String,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let ip: std::net::IpAddr = match bind.parse() {
            Ok(i) => i,
            Err(e) => {
                eprintln!("[RemoteServer] bind 地址解析失败 ({}): {}", bind, e);
                return;
            }
        };
        let addr = SocketAddr::new(ip, port);
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[RemoteServer] 绑定 {} 失败: {}", addr, e);
                return;
            }
        };
        println!("[RemoteServer] Mini HTTP API 已启动: http://{}", addr);

        let state = Arc::new(ApiState {
            store,
            secret,
            version,
        });

        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[RemoteServer] accept 失败: {}", e);
                    continue;
                }
            };
            let state = state.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(route(state, req, peer).await) }
                });
                if let Err(e) = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(io, service)
                    .await
                {
                    eprintln!("[RemoteServer] 连接错误: {}", e);
                }
            });
        }
    })
}

async fn route(
    state: Arc<ApiState>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<ResponseBody> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // /health 不需要鉴权
    if path == "/health" && method == Method::GET {
        return handle_health(&state);
    }

    // 其余路径统一鉴权
    if !check_auth(&state, req.headers()) {
        eprintln!("[RemoteServer] {} {} 401 from {}", method, path, peer);
        return json_resp(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}));
    }

    // 路由
    if path == "/accounts" {
        match method {
            Method::GET => return handle_list(&state),
            Method::POST => return handle_upsert(&state, req).await,
            _ => {}
        }
    }

    // /accounts/:id  /accounts/:id/token
    if let Some(rest) = path.strip_prefix("/accounts/") {
        let mut parts = rest.splitn(2, '/');
        let id = parts.next().unwrap_or("");
        let sub = parts.next();
        if !id.is_empty() {
            match (method.clone(), sub) {
                (Method::GET, Some("token")) => return handle_get_token(&state, id),
                (Method::GET, None) => return handle_get_account(&state, id),
                (Method::DELETE, None) => return handle_delete(&state, id),
                _ => {}
            }
        }
    }

    json_resp(StatusCode::NOT_FOUND, json!({"error": "not found"}))
}

fn check_auth(state: &ApiState, headers: &hyper::HeaderMap) -> bool {
    if state.secret.is_empty() {
        return false; // 未配置密钥时拒绝所有请求（避免误暴露）
    }
    match headers.get("X-Auth-Token").and_then(|v| v.to_str().ok()) {
        Some(v) if v == state.secret => true,
        _ => false,
    }
}

fn handle_health(state: &ApiState) -> Response<ResponseBody> {
    let count = state
        .store
        .lock()
        .map(|s| s.list_accounts().len())
        .unwrap_or(0);
    json_resp(
        StatusCode::OK,
        json!({
            "mode": "server",
            "version": state.version,
            "account_count": count,
        }),
    )
}

fn handle_list(state: &ApiState) -> Response<ResponseBody> {
    let store = match state.store.lock() {
        Ok(s) => s,
        Err(e) => return err_resp(format!("锁获取失败: {}", e)),
    };
    let accounts: Vec<Account> = store.list_accounts().into_iter().cloned().collect();
    json_resp(StatusCode::OK, json!({ "accounts": accounts }))
}

fn handle_get_account(state: &ApiState, id: &str) -> Response<ResponseBody> {
    let store = match state.store.lock() {
        Ok(s) => s,
        Err(e) => return err_resp(format!("锁获取失败: {}", e)),
    };
    match store.list_accounts().into_iter().find(|a| a.id == id) {
        Some(a) => json_resp(StatusCode::OK, json!({ "account": a })),
        None => json_resp(StatusCode::NOT_FOUND, json!({"error": "account not found"})),
    }
}

fn handle_get_token(state: &ApiState, id: &str) -> Response<ResponseBody> {
    let store = match state.store.lock() {
        Ok(s) => s,
        Err(e) => return err_resp(format!("锁获取失败: {}", e)),
    };
    match store.list_accounts().into_iter().find(|a| a.id == id) {
        Some(a) => json_resp(
            StatusCode::OK,
            json!({
                "auth_json": a.auth_json,
                "refresh_token": a.refresh_token,
            }),
        ),
        None => json_resp(StatusCode::NOT_FOUND, json!({"error": "account not found"})),
    }
}

#[derive(Deserialize)]
struct UpsertPayload {
    account: Account,
}

#[derive(Serialize)]
struct UpsertResult {
    ok: bool,
    id: String,
    upserted: &'static str,
}

async fn handle_upsert(state: &ApiState, req: Request<Incoming>) -> Response<ResponseBody> {
    let body = match req.collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return err_resp(format!("读取 body 失败: {}", e)),
    };
    let payload: UpsertPayload = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_resp(
                StatusCode::BAD_REQUEST,
                json!({"error": format!("JSON 解析失败: {}", e)}),
            );
        }
    };

    let mut store = match state.store.lock() {
        Ok(s) => s,
        Err(e) => return err_resp(format!("锁获取失败: {}", e)),
    };

    let id = payload.account.id.clone();
    let exists = store.list_accounts().iter().any(|a| a.id == id);
    let action = if exists { "updated" } else { "created" };

    if exists {
        // 用已有的 update_account（只覆盖可变字段），或直接替换整个账号
        // 为了简单：用 import 的方式修改 accounts 向量
        if let Err(e) = upsert_account(&mut store, payload.account) {
            return err_resp(e);
        }
    } else {
        // 直接 push
        if let Err(e) = upsert_account(&mut store, payload.account) {
            return err_resp(e);
        }
    }

    if let Err(e) = store.save() {
        return err_resp(format!("保存失败: {}", e));
    }

    let body = UpsertResult {
        ok: true,
        id,
        upserted: action,
    };
    match serde_json::to_vec(&body) {
        Ok(v) => resp_with_body(StatusCode::OK, v),
        Err(e) => err_resp(format!("序列化响应失败: {}", e)),
    }
}

/// 直接 upsert 到 accounts HashMap
fn upsert_account(store: &mut AccountStore, incoming: Account) -> Result<(), String> {
    store.accounts.insert(incoming.id.clone(), incoming);
    Ok(())
}

fn handle_delete(state: &ApiState, id: &str) -> Response<ResponseBody> {
    let mut store = match state.store.lock() {
        Ok(s) => s,
        Err(e) => return err_resp(format!("锁获取失败: {}", e)),
    };
    if let Err(e) = store.delete_account(id) {
        return json_resp(StatusCode::BAD_REQUEST, json!({"error": e}));
    }
    if let Err(e) = store.save() {
        return err_resp(format!("保存失败: {}", e));
    }
    json_resp(StatusCode::OK, json!({"ok": true}))
}

fn json_resp(status: StatusCode, value: Value) -> Response<ResponseBody> {
    match serde_json::to_vec(&value) {
        Ok(v) => resp_with_body(status, v),
        Err(_) => resp_with_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"{\"error\":\"json encode failed\"}".to_vec(),
        ),
    }
}

fn err_resp(msg: String) -> Response<ResponseBody> {
    json_resp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": msg}))
}

fn resp_with_body(status: StatusCode, body: Vec<u8>) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}
