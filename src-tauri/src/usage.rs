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
                if let Ok(token_res) = crate::oauth::refresh_access_token(rt).await {
                    current_token = token_res.access_token.clone();
                    new_tokens = Some(token_res);

                    // 重试请求
                    response = build_request(&current_token, &account_id)
                        .send()
                        .await
                        .map_err(|e| format!("刷新后重试失败: {}", e))?;
                    status = response.status();
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
}
