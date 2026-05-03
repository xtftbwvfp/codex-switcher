//! Sentinel token 生成 + 拉服务端 token。
//!
//! 参考实现：
//! - https://github.com/leetanshaj/openai-sentinel
//! - https://github.com/DestinyCycloid/chatgpt_register_v2_by_AI/blob/main/lib/clients.py
//!
//! 核心结论（针对 authorize_continue 流程）：
//! 1. 客户端先构造一个 19 元素的"伪浏览器环境" config 数组，base64(JSON) 后前缀 "gAAAAAC"
//!    = requirements_token (字段名 "p")
//! 2. POST https://sentinel.openai.com/backend-api/sentinel/req {"p":..., "id":<did>, "flow":...}
//!    服务端返回 {"token":"<server_token>", "turnstile":{...}, "proofofwork":{...}}
//! 3. 后续每次调 auth.openai.com/api/accounts/* 时，header 加：
//!    openai-sentinel-token: {"p":"","t":"","c":"<server_token>","id":<did>,"flow":<flow>}
//!
//! 注意：authorize_continue 流程下 p/t 可以为空字符串，只用 c 就能过。

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::{rng, Rng};
use serde_json::{json, Value};

pub const SENTINEL_REQ_URL: &str = "https://sentinel.openai.com/backend-api/sentinel/req";
pub const SENTINEL_FRAME_REFERER: &str =
    "https://sentinel.openai.com/backend-api/sentinel/frame.html?sv=20260219f9f6";
pub const SENTINEL_SDK_SCRIPT: &str =
    "https://sentinel.openai.com/sentinel/20260124ceb8/sdk.js";

/// 构造一个 19 元素 config 数组（仿浏览器环境）。
fn build_config(user_agent: &str) -> Vec<Value> {
    let mut r = rng();
    let nav_random1: f64 = r.random();
    let nav_random2: f64 = r.random();
    let perf_now: f64 = r.random_range(1000.0..50000.0);
    let now_ms: f64 = chrono::Utc::now().timestamp_millis() as f64;
    let time_origin = now_ms - perf_now;

    let date_str = chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string();

    let nav_props = [
        "vendorSub",
        "productSub",
        "vendor",
        "maxTouchPoints",
        "scheduling",
        "userActivation",
        "doNotTrack",
        "geolocation",
        "connection",
        "plugins",
        "mimeTypes",
        "pdfViewerEnabled",
        "webkitTemporaryStorage",
        "webkitPersistentStorage",
        "hardwareConcurrency",
        "cookieEnabled",
        "credentials",
        "mediaDevices",
        "permissions",
        "locks",
        "ink",
    ];
    let nav_prop = nav_props[r.random_range(0..nav_props.len())];
    let nav_val = format!("{nav_prop}\u{2212}undefined");

    let doc_keys = ["location", "implementation", "URL", "documentURI", "compatMode"];
    let win_keys = ["Object", "Function", "Array", "Number", "parseFloat", "undefined"];
    let cores = [4i64, 8, 12, 16];
    let hardware_concurrency = cores[r.random_range(0..cores.len())];
    let sid = uuid::Uuid::new_v4().to_string();

    vec![
        json!("1920x1080"),
        json!(date_str),
        json!(4_294_705_152u64),
        json!(nav_random1),
        json!(user_agent),
        json!(SENTINEL_SDK_SCRIPT),
        Value::Null,
        Value::Null,
        json!("en-US"),
        json!("en-US,en"),
        json!(nav_random2),
        json!(nav_val),
        json!(doc_keys[r.random_range(0..doc_keys.len())]),
        json!(win_keys[r.random_range(0..win_keys.len())]),
        json!(perf_now),
        json!(sid),
        json!(""),
        json!(hardware_concurrency),
        json!(time_origin),
    ]
}

/// 生成 requirements token (字段 "p")。
pub fn build_requirements_token(user_agent: &str) -> String {
    let mut config = build_config(user_agent);
    // index 3 / 9 是 PoW 占位符；不做真 PoW，按参考实现填假值即可
    config[3] = json!(1u32);
    let mut r = rng();
    config[9] = json!(r.random_range(5u32..=50u32));
    let s = serde_json::to_string(&config).unwrap_or_else(|_| "[]".to_string());
    let b64 = B64.encode(s.as_bytes());
    format!("gAAAAAC{b64}")
}

/// POST sentinel/req → 拿服务端 token。
pub async fn fetch_sentinel_token(
    client: &reqwest::Client,
    user_agent: &str,
    device_id: &str,
    flow: &str,
) -> Result<String, String> {
    let p = build_requirements_token(user_agent);
    let body = json!({"p": p, "id": device_id, "flow": flow}).to_string();

    let resp = client
        .post(SENTINEL_REQ_URL)
        .header("origin", "https://sentinel.openai.com")
        .header("referer", SENTINEL_FRAME_REFERER)
        .header("content-type", "text/plain;charset=UTF-8")
        .header("accept", "*/*")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("sentinel/req 请求失败: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("sentinel/req 读取响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("sentinel/req 返回 {status}: {}", &text[..text.len().min(200)]));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("sentinel/req JSON 解析失败: {e}"))?;
    v.get("token")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("sentinel/req 响应缺 token: {}", &text[..text.len().min(200)]))
}

/// 拼接 openai-sentinel-token 头部值。
pub fn make_sentinel_header(server_token: &str, device_id: &str, flow: &str) -> String {
    json!({
        "p": "",
        "t": "",
        "c": server_token,
        "id": device_id,
        "flow": flow,
    })
    .to_string()
}
