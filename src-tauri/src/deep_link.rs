//! Deep link 处理：codexswitch:// 与 ccswitch:// 兼容协议
//!
//! 支持的形态（任选其一）：
//! - `codexswitch://v1/import?resource=provider&app=codex&name=...&endpoint=...&apiKey=...`
//! - `ccswitch://v1/import?...`（cc-switch 协议字段，仅消费 `app=codex` 的 provider）
//!
//! 关键安全约束：
//! - `usageScript` 字段绝不直接执行；只算 SHA-256 与本地白名单比对，命中 → 映射到内置 fetcher preset
//! - `endpoint`/`base_url` 必须 `http(s)://`
//! - 解析结果必须经用户在 UI 弹窗确认才落库

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use url::Url;

/// 已知 usageScript 的 SHA-256 → 内置 fetcher preset 名映射。
///
/// 哈希值来自 cc-switch 生态里常见 provider 的脚本规范化（base64 解码后原样 hash）。
/// 命中 → 映射到 Rust 实现的内置 fetcher；不命中 → 弃用脚本，preset=None。
fn known_usage_script_presets() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    // cc-switch 通用 OpenAI 兼容 usage 脚本：GET {base}/v1/usage with Bearer，
    // extractor 取 remaining / quota.remaining / balance，单位 USD/CNY/...，
    // 等价于 Rust 端 `UsageFetcher::fetch_relay_usage_openai_compat`。
    // unity2.ai 等多家中转站默认下发的就是这段。
    m.insert(
        "8b143b665e6cb4b5531cfdfe6a27f20d04bf8449eda5d54a589c9affa8409db4",
        "openai_compat",
    );
    m
}

/// 解析后传给前端的待导入 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRelayImport {
    pub source: String, // "codexswitch" / "ccswitch"
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub homepage: Option<String>,
    pub usage_preset: Option<String>,
    /// 携带了 usageScript 但未命中白名单 → 提示用户该字段已忽略
    pub usage_script_unknown: bool,
}

#[derive(Debug)]
pub enum DeepLinkError {
    UnsupportedScheme,
    UnsupportedAction,
    NotForCodex,
    MissingField(&'static str),
    InvalidBaseUrl,
    InvalidApiKey,
}

impl std::fmt::Display for DeepLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme => write!(f, "URL scheme 不识别"),
            Self::UnsupportedAction => write!(f, "URL action 不识别（仅支持 /v1/import）"),
            Self::NotForCodex => write!(f, "ccswitch 链接 app != codex，已忽略"),
            Self::MissingField(name) => write!(f, "缺少必填字段: {}", name),
            Self::InvalidBaseUrl => write!(f, "endpoint/base_url 必须以 http:// 或 https:// 开头"),
            Self::InvalidApiKey => write!(f, "apiKey 字段无效"),
        }
    }
}

/// 解析一条 deep link URL，返回待用户确认的 import payload。
///
/// 不会落库、不会执行任何 usageScript。
pub fn parse(url: &str) -> Result<PendingRelayImport, DeepLinkError> {
    let parsed = Url::parse(url).map_err(|_| DeepLinkError::UnsupportedScheme)?;

    let scheme = parsed.scheme();
    let source = match scheme {
        "codexswitch" => "codexswitch".to_string(),
        "ccswitch" => "ccswitch".to_string(),
        _ => return Err(DeepLinkError::UnsupportedScheme),
    };

    // 形如 codexswitch://v1/import?... → host=v1, path=/import
    let host = parsed.host_str().unwrap_or("");
    let path = parsed.path();
    if host != "v1" || !path.starts_with("/import") {
        return Err(DeepLinkError::UnsupportedAction);
    }

    let params: HashMap<String, String> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // ccswitch 协议：只接受 app=codex（缺省也接受，宽松模式）
    if scheme == "ccswitch" {
        if let Some(app) = params.get("app") {
            if app != "codex" {
                return Err(DeepLinkError::NotForCodex);
            }
        }
    }

    // 字段映射：codexswitch 用 base_url；ccswitch 用 endpoint。统一成 base_url
    let name = params
        .get("name")
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or(DeepLinkError::MissingField("name"))?;

    let base_url = params
        .get("base_url")
        .or_else(|| params.get("endpoint"))
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or(DeepLinkError::MissingField("base_url/endpoint"))?;

    if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
        return Err(DeepLinkError::InvalidBaseUrl);
    }

    let api_key = params
        .get("api_key")
        .or_else(|| params.get("apiKey"))
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or(DeepLinkError::MissingField("api_key/apiKey"))?;

    if api_key.len() < 8 {
        return Err(DeepLinkError::InvalidApiKey);
    }

    let homepage = params.get("homepage").filter(|v| !v.is_empty()).cloned();

    // usage_preset 优先级：
    // 1) 显式 usage_preset 字段（codexswitch 推荐）
    // 2) ccswitch 的 usageScript → base64 解码 → SHA-256 → 白名单匹配
    let mut usage_preset = params
        .get("usage_preset")
        .cloned()
        .filter(|v| !v.is_empty());
    let mut usage_script_unknown = false;

    if usage_preset.is_none() {
        if let Some(script_b64) = params.get("usageScript").filter(|v| !v.is_empty()) {
            let usage_enabled = params
                .get("usageEnabled")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true);
            if usage_enabled {
                match base64::engine::general_purpose::STANDARD.decode(script_b64.as_bytes()) {
                    Ok(decoded) => {
                        let hash = Sha256::digest(&decoded);
                        let hex = hash.iter().fold(String::new(), |mut acc, b| {
                            use std::fmt::Write;
                            let _ = write!(acc, "{:02x}", b);
                            acc
                        });
                        match known_usage_script_presets().get(hex.as_str()) {
                            Some(preset) => usage_preset = Some((*preset).to_string()),
                            None => usage_script_unknown = true,
                        }
                    }
                    Err(_) => {
                        // base64 解码失败：当作未知脚本
                        usage_script_unknown = true;
                    }
                }
            }
        }
    }

    Ok(PendingRelayImport {
        source,
        name,
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
        homepage,
        usage_preset,
        usage_script_unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codexswitch_minimal() {
        let url = "codexswitch://v1/import?name=Unity2.Ai&base_url=https%3A%2F%2Funity2.ai&api_key=sk-abcdefg12345";
        let p = parse(url).expect("parse should succeed");
        assert_eq!(p.source, "codexswitch");
        assert_eq!(p.name, "Unity2.Ai");
        assert_eq!(p.base_url, "https://unity2.ai");
        assert_eq!(p.api_key, "sk-abcdefg12345");
        assert!(p.usage_preset.is_none());
        assert!(!p.usage_script_unknown);
    }

    #[test]
    fn parses_ccswitch_endpoint_alias() {
        let url = "ccswitch://v1/import?resource=provider&app=codex&name=Unity2.Ai&endpoint=https%3A%2F%2Funity2.ai&apiKey=sk-aaaaaaaaa";
        let p = parse(url).expect("parse should succeed");
        assert_eq!(p.source, "ccswitch");
        assert_eq!(p.base_url, "https://unity2.ai");
        assert_eq!(p.api_key, "sk-aaaaaaaaa");
    }

    #[test]
    fn ccswitch_app_filter_rejects_non_codex() {
        let url = "ccswitch://v1/import?resource=provider&app=claude&name=X&endpoint=https%3A%2F%2Fx.com&apiKey=sk-aaaaaaaa";
        assert!(matches!(parse(url), Err(DeepLinkError::NotForCodex)));
    }

    #[test]
    fn rejects_unknown_scheme() {
        let url = "javascript://v1/import?name=x";
        assert!(matches!(parse(url), Err(DeepLinkError::UnsupportedScheme)));
    }

    #[test]
    fn rejects_http_base_url_via_javascript_pseudo() {
        let url =
            "codexswitch://v1/import?name=X&base_url=javascript%3Aalert(1)&api_key=sk-aaaaaaaa";
        assert!(matches!(parse(url), Err(DeepLinkError::InvalidBaseUrl)));
    }

    #[test]
    fn unknown_usage_script_is_flagged_but_does_not_fail() {
        // 任意伪 base64 既未命中白名单 → usage_script_unknown=true，但解析仍成功
        let url = "ccswitch://v1/import?app=codex&name=Y&endpoint=https%3A%2F%2Fy.com&apiKey=sk-aaaaaaaa&usageEnabled=true&usageScript=AAAA";
        let p = parse(url).expect("parse should succeed");
        assert!(p.usage_preset.is_none());
        assert!(p.usage_script_unknown);
    }

    #[test]
    fn ccswitch_default_openai_compat_script_maps_to_preset() {
        // cc-switch 默认 OpenAI 兼容 usage 模板（unity2 等中转站下发）→ 命中 openai_compat
        let script_b64 = "KHsKICAgIHJlcXVlc3Q6IHsKICAgICAgdXJsOiAie3tiYXNlVXJsfX0vdjEvdXNhZ2UiLAogICAgICBtZXRob2Q6ICJHRVQiLAogICAgICBoZWFkZXJzOiB7ICJBdXRob3JpemF0aW9uIjogIkJlYXJlciB7e2FwaUtleX19IiB9CiAgICB9LAogICAgZXh0cmFjdG9yOiBmdW5jdGlvbihyZXNwb25zZSkgewogICAgICBjb25zdCByZW1haW5pbmcgPSByZXNwb25zZT8ucmVtYWluaW5nID8/IHJlc3BvbnNlPy5xdW90YT8ucmVtYWluaW5nID8/IHJlc3BvbnNlPy5iYWxhbmNlOwogICAgICBjb25zdCB1bml0ID0gcmVzcG9uc2U/LnVuaXQgPz8gcmVzcG9uc2U/LnF1b3RhPy51bml0ID8/ICJVU0QiOwogICAgICByZXR1cm4gewogICAgICAgIGlzVmFsaWQ6IHJlc3BvbnNlPy5pc19hY3RpdmUgPz8gcmVzcG9uc2U/LmlzVmFsaWQgPz8gdHJ1ZSwKICAgICAgICByZW1haW5pbmcsCiAgICAgICAgdW5pdAogICAgICB9OwogICAgfQogIH0p";
        let url = format!(
            "ccswitch://v1/import?app=codex&name=Z&endpoint=https%3A%2F%2Fz.com&apiKey=sk-aaaaaaaa&usageEnabled=true&usageScript={}",
            urlencoding::encode(script_b64),
        );
        let p = parse(&url).expect("parse should succeed");
        assert_eq!(p.usage_preset.as_deref(), Some("openai_compat"));
        assert!(!p.usage_script_unknown);
    }

    #[test]
    fn missing_required_field_errors() {
        let url = "codexswitch://v1/import?base_url=https%3A%2F%2Fx.com&api_key=sk-aaaaaaaa";
        assert!(matches!(parse(url), Err(DeepLinkError::MissingField(_))));
    }

    #[test]
    fn trailing_slash_on_base_url_is_normalized() {
        let url =
            "codexswitch://v1/import?name=X&base_url=https%3A%2F%2Fy.com%2F&api_key=sk-aaaaaaaa";
        let p = parse(url).unwrap();
        assert_eq!(p.base_url, "https://y.com");
    }
}
