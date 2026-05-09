//! 批量导入：自动嗅探 5 种第三方账号导出格式，统一转成 (email, auth_json) 入库。
//!
//! 支持格式：
//!   - cpa（codex_credentials zip）：每个账号一个 .json，根级 access_token / refresh_token /
//!     id_token / account_id / email / expired / last_refresh
//!   - sub2api：顶层 {exported_at, proxies, accounts[]}，每条 account.credentials.*
//!   - cockpit：顶层数组，每条 {email, tokens.{access/refresh/id}, account_id, plan_type}
//!   - 四段RT：每行 `email----xxx----xxx----rt_xxx`，只有 refresh_token
//!   - native：codex-switcher 自己导出的 accounts.json（原 import 路径）
//!
//! 入口：parse_any(content_bytes, filename) → Vec<ParsedAccount>
//!
//! 调用方拿到 ParsedAccount 列表后，用 store.add_account / merge 落库。

use base64::Engine as _;
use serde_json::{json, Value};
use std::io::Read;

#[derive(Debug, Clone)]
pub struct ParsedAccount {
    pub email: String,
    pub auth_json: Value,
    /// 自动登录链路用得上的元信息（plan_type 等）暂时不动 store schema，先丢这里
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    /// "rt-only" 标记 —— 只有 refresh_token，access_token 缺，需要后续 oauth refresh 才能用
    pub needs_refresh: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportSummary {
    pub format: String,
    pub parsed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BulkImportResult {
    pub summaries: Vec<ImportSummary>,
    pub accounts: Vec<BulkParsedAccountInfo>,
    /// 把所有 file-level error 也透出来（比如 zip 解压失败、JSON 解析失败）
    pub fatal: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BulkParsedAccountInfo {
    pub email: String,
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    pub needs_refresh: bool,
}

/// 入口：根据文件名 + 内容嗅探并分发。
/// `content_b64` 是文件内容的 base64 编码（前端读 binary 转 base64 传过来，
/// 文本 / json / zip 全部走同一通道）。
pub fn parse_one_file(
    filename: &str,
    content_b64: &str,
) -> Result<(String, Vec<ParsedAccount>), String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(content_b64.as_bytes())
        .map_err(|e| format!("base64 解码失败: {}", e))?;

    // 1) zip → cpa 格式（每个 entry 是一个独立账号 json）
    if filename.ends_with(".zip") || raw.starts_with(b"PK\x03\x04") {
        let accounts = parse_cpa_zip(&raw)?;
        return Ok(("cpa".to_string(), accounts));
    }

    let text = std::str::from_utf8(&raw).map_err(|_| "文件不是 UTF-8 文本")?;
    let trimmed = text.trim();

    // 2) JSON 类格式：sub2api / cockpit / cpa-单文件 / native accounts.json
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let v: Value =
            serde_json::from_str(trimmed).map_err(|e| format!("JSON 解析失败: {}", e))?;
        // sub2api：根对象有 "accounts" 数组 + "proxies" 字段（独有）
        if v.is_object()
            && v.get("accounts").map_or(false, |a| a.is_array())
            && v.get("proxies").is_some()
        {
            return Ok(("sub2api".to_string(), parse_sub2api(&v)?));
        }
        // cockpit：顶层是数组，每条有 tokens.access_token + email
        if let Some(arr) = v.as_array() {
            if arr.iter().any(|x| {
                x.get("tokens").is_some()
                    && x.get("email").is_some()
                    && x.get("auth_mode").is_some()
            }) {
                return Ok(("cockpit".to_string(), parse_cockpit(arr)?));
            }
            // 顶层数组但每个元素长得像 cpa 单条（access_token + refresh_token 在根级）
            if arr
                .iter()
                .all(|x| x.get("access_token").is_some() && x.get("refresh_token").is_some())
            {
                let parsed = arr
                    .iter()
                    .filter_map(|x| parse_cpa_single(x).ok())
                    .collect::<Vec<_>>();
                return Ok(("cpa".to_string(), parsed));
            }
        }
        // 单个对象 + 根级 access_token / refresh_token / email → cpa 单文件
        if v.get("access_token").is_some()
            && v.get("refresh_token").is_some()
            && v.get("email").is_some()
        {
            return Ok(("cpa".to_string(), vec![parse_cpa_single(&v)?]));
        }
        // 其他：当作 codex-switcher native accounts.json
        if v.get("accounts").is_some() {
            return Ok(("native".to_string(), parse_native(&v)?));
        }
        return Err("无法识别的 JSON 结构".to_string());
    }

    // 3) 文本格式：四段 RT
    if trimmed
        .lines()
        .any(|l| l.contains("----") && l.contains("rt_"))
    {
        return Ok((
            "four-segment-rt".to_string(),
            parse_four_segment_rt(trimmed)?,
        ));
    }

    Err("无法识别的文件格式".to_string())
}

/// cpa zip：解压每个 .json entry，每个是一个 cpa 单条
fn parse_cpa_zip(bytes: &[u8]) -> Result<Vec<ParsedAccount>, String> {
    let reader = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("zip 解析失败: {}", e))?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| format!("zip entry {} 失败: {}", i, e))?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        if !name.to_lowercase().ends_with(".json") {
            continue;
        }
        let mut content = String::new();
        if entry.read_to_string(&mut content).is_err() {
            continue;
        }
        match serde_json::from_str::<Value>(&content)
            .map_err(|e| format!("{} 解析 JSON 失败: {}", name, e))
            .and_then(|v| parse_cpa_single(&v))
        {
            Ok(acc) => out.push(acc),
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// cpa 单条：access_token / refresh_token / id_token / email / account_id / expired
fn parse_cpa_single(v: &Value) -> Result<ParsedAccount, String> {
    let email = v
        .get("email")
        .and_then(|x| x.as_str())
        .ok_or("缺 email")?
        .to_string();
    let access_token = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or("缺 access_token")?;
    let refresh_token = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .ok_or("缺 refresh_token")?;
    let id_token = v.get("id_token").and_then(|x| x.as_str()).unwrap_or("");
    let account_id = v.get("account_id").and_then(|x| x.as_str()).unwrap_or("");
    let expires_at = v
        .get("expired")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let last_refresh = v
        .get("last_refresh")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let auth_json = json!({
        "tokens": {
            "access_token": access_token,
            "refresh_token": refresh_token,
            "id_token": id_token,
            "account_id": account_id,
            "expires_at": expires_at,
        },
        "last_refresh": last_refresh,
    });
    Ok(ParsedAccount {
        email,
        auth_json,
        plan_type: None,
        account_id: if account_id.is_empty() {
            None
        } else {
            Some(account_id.to_string())
        },
        needs_refresh: false,
    })
}

/// sub2api：accounts[].credentials.*
fn parse_sub2api(v: &Value) -> Result<Vec<ParsedAccount>, String> {
    let arr = v
        .get("accounts")
        .and_then(|x| x.as_array())
        .ok_or("无 accounts 数组")?;
    let mut out = Vec::new();
    for acc in arr {
        let name = acc
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let cred = acc
            .get("credentials")
            .ok_or_else(|| format!("{} 缺 credentials", name))?;
        let access_token = match cred.get("access_token").and_then(|x| x.as_str()) {
            Some(s) => s,
            None => continue, // 没 access_token 跳过（rt-only 应走四段 RT 路径）
        };
        let refresh_token = cred
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let id_token = cred.get("id_token").and_then(|x| x.as_str()).unwrap_or("");
        let email = cred
            .get("email")
            .and_then(|x| x.as_str())
            .unwrap_or(&name)
            .to_string();
        let account_id = cred
            .get("chatgpt_account_id")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let plan_type = cred
            .get("plan_type")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let expires_at = cred.get("expires_at").and_then(|x| x.as_i64()).map(|ts| {
            chrono::DateTime::from_timestamp(ts, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default()
        });

        let auth_json = json!({
            "tokens": {
                "access_token": access_token,
                "refresh_token": refresh_token,
                "id_token": id_token,
                "account_id": account_id,
                "expires_at": expires_at,
            },
            "last_refresh": chrono::Utc::now().to_rfc3339(),
        });
        out.push(ParsedAccount {
            email,
            auth_json,
            plan_type,
            account_id: if account_id.is_empty() {
                None
            } else {
                Some(account_id.to_string())
            },
            needs_refresh: refresh_token.is_empty(),
        });
    }
    Ok(out)
}

/// cockpit：顶层数组，每条 {email, tokens.{access/refresh/id}, account_id, plan_type}
fn parse_cockpit(arr: &[Value]) -> Result<Vec<ParsedAccount>, String> {
    let mut out = Vec::new();
    for entry in arr {
        let email = match entry.get("email").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let tokens = match entry.get("tokens") {
            Some(t) => t,
            None => continue,
        };
        let access_token = tokens
            .get("access_token")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let refresh_token = tokens
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let id_token = tokens
            .get("id_token")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let account_id = entry
            .get("account_id")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let plan_type = entry
            .get("plan_type")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        if access_token.is_empty() && refresh_token.is_empty() {
            continue;
        }

        let auth_json = json!({
            "tokens": {
                "access_token": access_token,
                "refresh_token": refresh_token,
                "id_token": id_token,
                "account_id": account_id,
                "expires_at": null,
            },
            "last_refresh": chrono::Utc::now().to_rfc3339(),
        });
        out.push(ParsedAccount {
            email,
            auth_json,
            plan_type,
            account_id: if account_id.is_empty() {
                None
            } else {
                Some(account_id.to_string())
            },
            needs_refresh: access_token.is_empty(),
        });
    }
    Ok(out)
}

/// 四段 RT：每行 `email----xxx----xxx----rt_xxx`
fn parse_four_segment_rt(text: &str) -> Result<Vec<ParsedAccount>, String> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split("----").collect();
        if parts.len() < 4 {
            continue;
        }
        let email = parts[0].trim().to_string();
        let rt = parts[parts.len() - 1].trim().to_string();
        if email.is_empty() || rt.is_empty() || !rt.starts_with("rt_") {
            continue;
        }
        // access_token 缺 → access_token=""，extract_refresh_token 仍能拿到 rt；
        // 第一次请求会经 silent_refresh 路径自动补 access_token。
        let auth_json = json!({
            "tokens": {
                "access_token": "",
                "refresh_token": rt,
                "id_token": "",
                "account_id": "",
                "expires_at": null,
            },
            "last_refresh": chrono::Utc::now().to_rfc3339(),
        });
        out.push(ParsedAccount {
            email,
            auth_json,
            plan_type: None,
            account_id: None,
            needs_refresh: true,
        });
        let _ = idx;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn detects_sub2api() {
        let body = br#"{"exported_at":"2026-05-06","proxies":[],"accounts":[{"name":"a@x.com","credentials":{"access_token":"at","refresh_token":"rt","id_token":"id","email":"a@x.com","plan_type":"plus","chatgpt_account_id":"acc1","expires_at":1778898555}}]}"#;
        let (fmt, parsed) = parse_one_file("sub2api-import.json", &b64(body)).unwrap();
        assert_eq!(fmt, "sub2api");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].email, "a@x.com");
        assert_eq!(parsed[0].plan_type.as_deref(), Some("plus"));
        assert!(!parsed[0].needs_refresh);
    }

    #[test]
    fn detects_cockpit() {
        let body = br#"[{"id":"x","email":"b@x.com","auth_mode":"oauth","tokens":{"access_token":"at","refresh_token":"rt","id_token":"id"},"account_id":"acc","plan_type":"pro"}]"#;
        let (fmt, parsed) = parse_one_file("cockpit-import.json", &b64(body)).unwrap();
        assert_eq!(fmt, "cockpit");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].email, "b@x.com");
        assert_eq!(parsed[0].plan_type.as_deref(), Some("pro"));
    }

    #[test]
    fn detects_four_segment_rt() {
        let body = b"a@x.com----xxx----xxx----rt_aaa.bbb\nb@x.com----yyy----yyy----rt_ccc.ddd";
        let (fmt, parsed) = parse_one_file("rt.txt", &b64(body)).unwrap();
        assert_eq!(fmt, "four-segment-rt");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].email, "a@x.com");
        assert!(parsed[0].needs_refresh);
        assert_eq!(
            parsed[0]
                .auth_json
                .get("tokens")
                .and_then(|t| t.get("refresh_token"))
                .and_then(|v| v.as_str()),
            Some("rt_aaa.bbb")
        );
    }

    /// 用真实文件做 end-to-end 验证（cargo test --lib bulk_import -- --ignored real_files）
    #[test]
    #[ignore]
    fn real_files() {
        let cases = [
            (
                "/Users/xiaojian/Downloads/chrome/sub2api-import.json",
                "sub2api",
            ),
            (
                "/Users/xiaojian/Downloads/chrome/cockpit-import.json",
                "cockpit",
            ),
            (
                "/Users/xiaojian/Downloads/chrome/accounts_refresh_tokens.txt",
                "four-segment-rt",
            ),
            (
                "/Users/xiaojian/Downloads/chrome/codex_credentials_2026-05-06_10-30-01_CST.zip",
                "cpa",
            ),
        ];
        for (path, expected_fmt) in cases {
            let raw = std::fs::read(path).expect(&format!("read {}", path));
            let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
            let filename = std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            let (fmt, parsed) = parse_one_file(filename, &b64).expect(&format!("parse {}", path));
            assert_eq!(fmt, expected_fmt, "format mismatch for {}", path);
            assert!(!parsed.is_empty(), "no accounts parsed from {}", path);
            println!("✓ {} → {} accounts ({})", filename, parsed.len(), fmt);
            for p in &parsed {
                println!(
                    "    {} plan={:?} needs_refresh={}",
                    p.email, p.plan_type, p.needs_refresh
                );
            }
        }
    }

    #[test]
    fn detects_cpa_single() {
        let body = br#"{"type":"codex","email":"c@x.com","access_token":"at","refresh_token":"rt","id_token":"id","account_id":"acc","expired":"2026-05-16T10:29:15+08:00","last_refresh":"2026-05-06T10:29:15+08:00"}"#;
        let (fmt, parsed) = parse_one_file("c.json", &b64(body)).unwrap();
        assert_eq!(fmt, "cpa");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].email, "c@x.com");
    }
}

/// codex-switcher native accounts.json（兼容现有 import 路径，让一个统一入口能吃所有）
fn parse_native(v: &Value) -> Result<Vec<ParsedAccount>, String> {
    let accounts = v
        .get("accounts")
        .and_then(|a| a.as_object())
        .ok_or("native: 缺 accounts 对象")?;
    let mut out = Vec::new();
    for (_id, acc) in accounts {
        let email = acc
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let auth_json = acc.get("auth_json").cloned().unwrap_or(Value::Null);
        if email.is_empty() || auth_json.is_null() {
            continue;
        }
        let needs_refresh = auth_json
            .get("tokens")
            .and_then(|t| t.get("access_token"))
            .and_then(|x| x.as_str())
            .map(|s| s.is_empty())
            .unwrap_or(true);
        out.push(ParsedAccount {
            email,
            auth_json,
            plan_type: None,
            account_id: None,
            needs_refresh,
        });
    }
    Ok(out)
}
