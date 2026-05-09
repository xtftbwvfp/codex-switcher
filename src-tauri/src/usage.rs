//! Codex Switcher - 用量获取模块
//!
//! 从 OpenAI API 获取 Codex 使用量信息

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 前端展示的用量数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDisplay {
    /// 套餐类型
    pub plan_type: String,
    /// 5小时窗口使用百分比
    pub five_hour_used: i32,
    /// 5小时窗口剩余百分比
    pub five_hour_left: i32,
    /// 5小时窗口标签 (如 "5H 限额")
    pub five_hour_label: String,
    /// 5小时重置时间描述
    pub five_hour_reset: String,
    /// 5小时重置时间戳
    pub five_hour_reset_at: Option<i64>,
    /// 周窗口使用百分比
    pub weekly_used: i32,
    /// 周窗口剩余百分比
    pub weekly_left: i32,
    /// 周窗口标签 (如 "周限额")
    pub weekly_label: String,
    /// 周重置时间描述
    pub weekly_reset: String,
    /// 周重置时间戳
    pub weekly_reset_at: Option<i64>,
    /// 额度余额
    pub credits_balance: Option<f64>,
    /// 是否有额度
    pub has_credits: bool,
    /// Token 是否对 CLI 有效 (api.openai.com)
    pub is_valid_for_cli: bool,
}

/// 用量获取器
pub struct UsageFetcher;

impl UsageFetcher {
    /// 从 API 获取用量 (直接使用提供的 Token，不读取 auth.json)
    pub async fn fetch_usage_direct(
        access_token: String,
        account_id: Option<String>,
        refresh_token: Option<String>,
        allow_local_refresh: bool,
    ) -> Result<(UsageDisplay, Option<crate::oauth::TokenResponse>), String> {
        let mut current_token = access_token;
        let mut new_tokens: Option<crate::oauth::TokenResponse> = None;

        let client = reqwest::Client::new();
        let user_agent = format!(
            "codex_cli_rs/{} (Mac OS; x86_64) codex-cli",
            env!("CARGO_PKG_VERSION")
        );
        let build_request = |at: &str, aid: &Option<String>| {
            let mut req = client
                .get("https://chatgpt.com/backend-api/wham/usage")
                .header("Authorization", format!("Bearer {}", at))
                .header("User-Agent", &user_agent)
                .header("originator", "codex_cli_rs")
                .header("Accept", "application/json")
                .timeout(std::time::Duration::from_secs(30));
            if let Some(id) = aid {
                req = req.header("ChatGPT-Account-Id", id);
            }
            req
        };

        let mut response = build_request(&current_token, &account_id)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {}", e))?;

        let mut status = response.status();

        // 如果允许本地刷新，且 401/403 且有 refresh_token，尝试刷新
        if allow_local_refresh && (status == 401 || status == 403) && refresh_token.is_some() {
            if let Some(ref rt) = refresh_token {
                match crate::oauth::refresh_access_token(rt).await {
                    Ok(token_res) => {
                        current_token = token_res.access_token.clone();
                        new_tokens = Some(token_res);

                        // 重试请求
                        response = build_request(&current_token, &account_id)
                            .send()
                            .await
                            .map_err(|e| format!("刷新后重试失败: {}", e))?;
                        status = response.status();
                    }
                    Err(e) => {
                        let lower = e.to_lowercase();
                        if lower.contains("logged out")
                            || lower.contains("signed in to another account")
                            || lower.contains("invalid_grant")
                        {
                            return Err("ACCOUNT_LOGGED_OUT:您已登出或登录了其他账号，请重新登录"
                                .to_string());
                        }
                    }
                }
            }
        }

        if status == 401 || status == 403 {
            // 读取响应体以检测是否为封号
            let body = response.text().await.unwrap_or_default().to_lowercase();
            let is_banned = body.contains("deactivated")
                || body.contains("banned")
                || body.contains("suspended")
                || body.contains("account_deactivated");

            if is_banned {
                return Err("ACCOUNT_BANNED:该账号已被封禁".to_string());
            }

            if !allow_local_refresh {
                return Err(
                    "当前激活账号访问配额接口返回 401/403；已禁用本地 refresh_token 刷新，请稍后重试或在 Codex 中触发一次请求".to_string(),
                );
            }
            // 如果刷新后仍然 401/403，标记为无效
            return Err("TOKEN_INVALID:授权已失效，请删除该账号后重新登录".to_string());
        }

        let text = response
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {}", e))?;

        let json: Value =
            serde_json::from_str(&text).map_err(|e| format!("解析 JSON 失败: {}", e))?;

        // 检测 200 状态码下的软封号/停用响应，如 {"detail":{"code":"deactivated_workspace"}}
        if let Some(detail_code) = json
            .get("detail")
            .and_then(|d| d.get("code"))
            .and_then(|c| c.as_str())
        {
            let code_lower = detail_code.to_lowercase();
            if code_lower.contains("deactivated")
                || code_lower.contains("banned")
                || code_lower.contains("suspended")
            {
                println!("[Usage] 检测到账号停用: detail.code={}", detail_code);
                return Err("ACCOUNT_BANNED:该账号已被封禁(workspace 已停用)".to_string());
            }
        }

        let display = Self::parse_usage_response(&json)?;

        Ok((display, new_tokens))
    }

    /// 从 Value 解析用量数据
    fn parse_usage_response(json: &Value) -> Result<UsageDisplay, String> {
        let plan_type = json
            .get("plan_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let rate_limit = json.get("rate_limit");

        // 解析 5 小时窗口 (Primary)
        let primary_val = rate_limit.and_then(|r| r.get("primary_window"));
        let (p_used, p_reset, p_label, p_reset_at) = Self::parse_window(primary_val, "5H 限额");

        // 解析周窗口 (Secondary)
        let secondary_val = rate_limit.and_then(|r| r.get("secondary_window"));
        let (s_used, s_reset, s_label, s_reset_at) = Self::parse_window(secondary_val, "周限额");

        // 解析额度
        let credits = json.get("credits");
        let has_credits = credits
            .and_then(|c| c.get("has_credits"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let unlimited = credits
            .and_then(|c| c.get("unlimited"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let credits_balance = credits
            .and_then(|c| c.get("balance"))
            .and_then(Self::parse_number);

        Ok(UsageDisplay {
            plan_type,
            five_hour_used: p_used,
            five_hour_left: 100 - p_used,
            five_hour_label: p_label,
            five_hour_reset: p_reset,
            five_hour_reset_at: p_reset_at,
            weekly_used: s_used,
            weekly_left: 100 - s_used,
            weekly_label: s_label,
            weekly_reset: s_reset,
            weekly_reset_at: s_reset_at,
            credits_balance,
            has_credits: has_credits || unlimited,
            is_valid_for_cli: true,
        })
    }

    /// 解析窗口数据
    fn parse_window(
        window: Option<&Value>,
        default_label: &str,
    ) -> (i32, String, String, Option<i64>) {
        let window = match window {
            Some(w) => w,
            None => return (0, "未知".to_string(), default_label.to_string(), None),
        };

        // 关键修复：使用 f64 解析百分比，然后四舍五入
        let used_percent = window
            .get("used_percent")
            .and_then(Self::parse_number)
            .map(|f| f.round() as i32)
            .unwrap_or(0);

        let reset_at = window
            .get("reset_at")
            .and_then(Self::parse_number)
            .map(|f| f as i64);

        let limit_window_seconds = window
            .get("limit_window_seconds")
            .and_then(Self::parse_number)
            .map(|f| f as i64)
            .unwrap_or(0);

        // 动态计算标签
        let label = if limit_window_seconds > 0 {
            Self::get_limits_label(limit_window_seconds)
        } else {
            default_label.to_string()
        };

        let reset_str = if let Some(ts) = reset_at {
            if ts > 0 {
                Self::format_reset(ts)
            } else {
                "未知".to_string()
            }
        } else {
            // 尝试使用 reset_after_seconds
            let reset_after = window
                .get("reset_after_seconds")
                .or_else(|| window.get("reset_after_sec"))
                .and_then(Self::parse_number)
                .map(|f| f as i64)
                .unwrap_or(0);
            if reset_after > 0 {
                Self::format_duration(reset_after)
            } else {
                "未知".to_string()
            }
        };

        (used_percent, reset_str, label, reset_at)
    }

    /// 根据窗口秒数获取人类可读标签
    fn get_limits_label(seconds: i64) -> String {
        const SECS_PER_HOUR: i64 = 3600;
        const SECS_PER_DAY: i64 = 24 * SECS_PER_HOUR;
        const SECS_PER_WEEK: i64 = 7 * SECS_PER_DAY;

        if seconds <= SECS_PER_HOUR * 5 + 600 {
            "5H 限额".to_string()
        } else if seconds <= SECS_PER_DAY + 600 {
            "24H 限额".to_string()
        } else if seconds <= SECS_PER_WEEK + 3600 {
            "周限额".to_string()
        } else {
            format!("{}H 限额", (seconds + 3599) / 3600)
        }
    }

    /// 解析数字（支持字符串和数字）
    fn parse_number(v: &Value) -> Option<f64> {
        match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// 解析整数（支持字符串和数字）
    fn parse_int(v: &Value) -> Option<i32> {
        match v {
            Value::Number(n) => n.as_i64().map(|i| i as i32),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// 格式化重置时间（时间戳）
    fn format_reset(reset_at: i64) -> String {
        use chrono::{TimeZone, Utc};

        if reset_at == 0 {
            return "未知".to_string();
        }

        let reset_time = Utc
            .timestamp_opt(reset_at, 0)
            .single()
            .unwrap_or_else(Utc::now);
        let now = Utc::now();

        let duration = reset_time.signed_duration_since(now);
        Self::format_chrono_duration(duration)
    }

    /// 格式化持续时间（秒）
    fn format_duration(seconds: i64) -> String {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;

        if hours > 24 {
            let days = hours / 24;
            format!("{}天后重置", days)
        } else if hours > 0 {
            format!("{}小时{}分钟后重置", hours, minutes)
        } else if minutes > 0 {
            format!("{}分钟后重置", minutes)
        } else {
            "即将重置".to_string()
        }
    }

    /// 格式化 chrono Duration
    fn format_chrono_duration(duration: chrono::Duration) -> String {
        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;

        if hours > 24 {
            let days = hours / 24;
            format!("{}天后重置", days)
        } else if hours > 0 {
            format!("{}小时{}分钟后重置", hours, minutes.abs())
        } else if minutes > 0 {
            format!("{}分钟后重置", minutes)
        } else {
            "即将重置".to_string()
        }
    }

    /// 中转站 OpenAI 兼容 usage：`GET {base}/v1/usage` with `Authorization: Bearer <key>`
    ///
    /// 字段优先级（cc-switch 通用模板兼容）：
    /// - remaining: `remaining` / `quota.remaining` / `balance`
    /// - unit:      `unit` / `quota.unit` / 默认 `"USD"`
    /// - is_active: `is_active` / `isValid` / 默认 `true`
    pub async fn fetch_relay_usage_openai_compat(
        base_url: &str,
        api_key: &str,
    ) -> Result<crate::account::RelayUsageCache, String> {
        let url = format!("{}/v1/usage", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("usage 请求失败: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            // 带上 URL + body 前 200 字节，方便定位（如 GLM 401 会返回中文"令牌过期"）
            let body_preview = resp
                .text()
                .await
                .map(|s| s.chars().take(200).collect::<String>())
                .unwrap_or_default();
            return Err(format!(
                "HTTP {} @ {} → {}",
                status.as_u16(),
                url,
                body_preview
            ));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("usage JSON 解析失败: {}", e))?;

        let remaining = body
            .get("remaining")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                body.get("quota")
                    .and_then(|q| q.get("remaining"))
                    .and_then(|v| v.as_f64())
            })
            .or_else(|| body.get("balance").and_then(|v| v.as_f64()))
            .ok_or_else(|| "上游响应缺 remaining/balance 字段".to_string())?;

        let unit = body
            .get("unit")
            .and_then(|v| v.as_str())
            .or_else(|| {
                body.get("quota")
                    .and_then(|q| q.get("unit"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("USD")
            .to_string();

        let is_active = body
            .get("is_active")
            .and_then(|v| v.as_bool())
            .or_else(|| body.get("isValid").and_then(|v| v.as_bool()))
            .unwrap_or(true);

        Ok(crate::account::RelayUsageCache {
            remaining,
            unit,
            is_active,
            next_reset_at: None,
            updated_at: chrono::Utc::now(),
        })
    }

    /// GLM / 智谱 monitor quota：`GET https://<host>/api/monitor/usage/quota/limit` with Bearer。
    ///
    /// 输入 `base_url` 通常是 OpenAI 兼容根（如 `https://open.bigmodel.cn/api/paas/v4`），
    /// 这里只取 origin，再拼 `/api/monitor/usage/quota/limit`。
    ///
    /// 响应：
    /// ```json
    /// {"code":200,"data":{"limits":[
    ///   {"type":"TIME_LIMIT","percentage":30,"remaining":0.7,"nextResetTime":1234567890000},
    ///   {"type":"TOKENS_LIMIT","percentage":50}
    /// ]}}
    /// ```
    /// 我们把 TOKENS_LIMIT 的 `100 - percentage` 当 `remaining`，单位 `"% tokens"`。
    pub async fn fetch_relay_usage_glm_zhipu(
        base_url: &str,
        api_key: &str,
    ) -> Result<crate::account::RelayUsageCache, String> {
        let origin = url::Url::parse(base_url)
            .ok()
            .and_then(|u| {
                u.host_str().map(|h| {
                    let scheme = u.scheme();
                    let port = u.port().map(|p| format!(":{}", p)).unwrap_or_default();
                    format!("{}://{}{}", scheme, h, port)
                })
            })
            .ok_or_else(|| format!("无法从 base_url 解析 origin: {}", base_url))?;

        let url = format!("{}/api/monitor/usage/quota/limit", origin);
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("usage 请求失败: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            let body_preview = resp
                .text()
                .await
                .map(|s| s.chars().take(200).collect::<String>())
                .unwrap_or_default();
            return Err(format!(
                "HTTP {} @ {} → {}",
                status.as_u16(),
                url,
                body_preview
            ));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("usage JSON 解析失败: {}", e))?;

        let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 200 {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("non-200 code");
            return Err(format!("GLM code={} msg={}", code, msg));
        }

        // 从 limits 数组里捞 TOKENS_LIMIT 的 percentage
        let tokens_pct = body
            .get("data")
            .and_then(|d| d.get("limits"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|lim| {
                    let kind = lim.get("type").and_then(|v| v.as_str())?;
                    if kind == "TOKENS_LIMIT" {
                        lim.get("percentage").and_then(|v| v.as_f64())
                    } else {
                        None
                    }
                })
            });

        // 计算 remaining 百分比（用 tokens 维度；优先级：TOKENS_LIMIT > TIME_LIMIT > 100）
        let used_pct = tokens_pct
            .or_else(|| {
                body.get("data")
                    .and_then(|d| d.get("limits"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find_map(|lim| lim.get("percentage").and_then(|v| v.as_f64()))
                    })
            })
            .unwrap_or(0.0);
        let remaining_pct = (100.0 - used_pct).max(0.0);

        // 取 nextResetTime（毫秒时间戳）→ Unix 秒
        let next_reset_at = body
            .get("data")
            .and_then(|d| d.get("limits"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find_map(|lim| lim.get("nextResetTime").and_then(|v| v.as_i64()))
            })
            .map(|ms| ms / 1000);

        Ok(crate::account::RelayUsageCache {
            remaining: remaining_pct,
            unit: "%".to_string(),
            is_active: remaining_pct > 0.0,
            next_reset_at,
            updated_at: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_relay_response(body: Value) -> Result<(f64, String, bool), String> {
        // 单元测试只验证字段优先级，不实际打网络
        let remaining = body
            .get("remaining")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                body.get("quota")
                    .and_then(|q| q.get("remaining"))
                    .and_then(|v| v.as_f64())
            })
            .or_else(|| body.get("balance").and_then(|v| v.as_f64()))
            .ok_or_else(|| "missing".to_string())?;
        let unit = body
            .get("unit")
            .and_then(|v| v.as_str())
            .or_else(|| {
                body.get("quota")
                    .and_then(|q| q.get("unit"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("USD")
            .to_string();
        let is_active = body
            .get("is_active")
            .and_then(|v| v.as_bool())
            .or_else(|| body.get("isValid").and_then(|v| v.as_bool()))
            .unwrap_or(true);
        Ok((remaining, unit, is_active))
    }

    #[test]
    fn relay_usage_top_level_fields() {
        let body = json!({"remaining": 12.5, "unit": "USD", "is_active": true});
        let (r, u, a) = parse_relay_response(body).unwrap();
        assert_eq!(r, 12.5);
        assert_eq!(u, "USD");
        assert!(a);
    }

    #[test]
    fn relay_usage_nested_quota() {
        let body = json!({"quota": {"remaining": 8.0, "unit": "CNY"}, "isValid": false});
        let (r, u, a) = parse_relay_response(body).unwrap();
        assert_eq!(r, 8.0);
        assert_eq!(u, "CNY");
        assert!(!a);
    }

    #[test]
    fn relay_usage_balance_alias_with_default_unit() {
        let body = json!({"balance": 1.23});
        let (r, u, a) = parse_relay_response(body).unwrap();
        assert_eq!(r, 1.23);
        assert_eq!(u, "USD"); // 默认
        assert!(a); // 默认
    }

    #[test]
    fn relay_usage_missing_remaining_errors() {
        let body = json!({"unit": "USD"});
        assert!(parse_relay_response(body).is_err());
    }

    fn parse_glm_quota(body: Value) -> Option<f64> {
        // 模拟 fetch_relay_usage_glm_zhipu 的解析步骤（不打网络）
        let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 200 {
            return None;
        }
        let used_pct = body
            .get("data")
            .and_then(|d| d.get("limits"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|lim| {
                    let kind = lim.get("type").and_then(|v| v.as_str())?;
                    if kind == "TOKENS_LIMIT" {
                        lim.get("percentage").and_then(|v| v.as_f64())
                    } else {
                        None
                    }
                })
            })?;
        Some((100.0 - used_pct).max(0.0))
    }

    #[test]
    fn glm_quota_picks_tokens_limit_percentage() {
        let body = json!({
            "code": 200,
            "data": {
                "limits": [
                    {"type": "TIME_LIMIT", "percentage": 30, "remaining": 0.7},
                    {"type": "TOKENS_LIMIT", "percentage": 25}
                ]
            }
        });
        let pct = parse_glm_quota(body).unwrap();
        assert_eq!(pct, 75.0); // 100 - 25
    }

    #[test]
    fn glm_quota_skips_when_code_not_200() {
        let body = json!({"code": 401, "message": "unauthorized"});
        assert!(parse_glm_quota(body).is_none());
    }
}
