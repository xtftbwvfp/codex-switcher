//! Codex Switcher - 用量获取模块
//! 
//! 从 OpenAI API 获取 Codex 使用量信息

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// 前端展示的用量数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDisplay {
    /// 套餐类型
    pub plan_type: String,
    /// 5小时窗口使用百分比
    pub five_hour_used: i32,
    /// 5小时窗口剩余百分比
    pub five_hour_left: i32,
    /// 5小时重置时间描述
    pub five_hour_reset: String,
    /// 5小时重置时间戳
    pub five_hour_reset_at: Option<i64>,
    /// 周窗口使用百分比
    pub weekly_used: i32,
    /// 周窗口剩余百分比
    pub weekly_left: i32,
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

/// Auth.json tokens 结构
#[derive(Debug, Clone, Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

/// Auth.json 结构
#[derive(Debug, Clone, Deserialize)]
struct AuthJson {
    tokens: Option<AuthTokens>,
}

/// 用量获取器
pub struct UsageFetcher;

impl UsageFetcher {
    /// 获取 Codex auth.json 路径
    fn auth_path() -> PathBuf {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".codex")
            .join("auth.json")
    }

    /// 读取认证信息
    pub fn read_auth() -> Result<(String, Option<String>), String> {
        let path = Self::auth_path();
        if !path.exists() {
            return Err("未找到 Codex auth.json，请先登录 Codex".to_string());
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| format!("读取 auth.json 失败: {}", e))?;

        let auth: AuthJson = serde_json::from_str(&content)
            .map_err(|e| format!("解析 auth.json 失败: {}", e))?;

        let tokens = auth.tokens
            .ok_or_else(|| "auth.json 中没有 tokens 字段".to_string())?;

        let token = tokens.access_token
            .ok_or_else(|| "auth.json 中没有 access_token".to_string())?;

        Ok((token, tokens.account_id))
    }

    /// 从 API 获取用量 (直接使用提供的 Token，不读取 auth.json)
    pub async fn fetch_usage_direct(
        access_token: String,
        account_id: Option<String>,
        refresh_token: Option<String>,
    ) -> Result<(UsageDisplay, Option<crate::oauth::TokenResponse>), String> {
        let mut current_token = access_token;
        let mut new_tokens: Option<crate::oauth::TokenResponse> = None;

        let client = reqwest::Client::new();
        let build_request = |at: &str, aid: &Option<String>| {
            let mut req = client
                .get("https://chatgpt.com/backend-api/wham/usage")
                .header("Authorization", format!("Bearer {}", at))
                .header("User-Agent", "CodexSwitcher/1.0")
                .header("Accept", "application/json")
                .timeout(std::time::Duration::from_secs(30));
            if let Some(id) = aid {
                req = req.header("ChatGPT-Account-Id", id);
            }
            req
        };

        let mut response = build_request(&current_token, &account_id).send().await
            .map_err(|e| format!("网络请求失败: {}", e))?;

        let mut status = response.status();
        
        // 如果 401/403 且有 refresh_token，尝试刷新
        if (status == 401 || status == 403) && refresh_token.is_some() {
            if let Some(ref rt) = refresh_token {
                if let Ok(token_res) = crate::oauth::refresh_access_token(rt).await {
                    current_token = token_res.access_token.clone();
                    new_tokens = Some(token_res);
                    
                    // 重试请求
                    response = build_request(&current_token, &account_id).send().await
                        .map_err(|e| format!("刷新后重试失败: {}", e))?;
                    status = response.status();
                }
            }
        }

        if status == 401 || status == 403 {
            // 如果刷新后仍然 401/403，标记为无效
            return Err("TOKEN_INVALID:授权已失效，请删除该账号后重新登录".to_string());
        }

        let text = response.text().await
            .map_err(|e| format!("读取响应失败: {}", e))?;

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("解析 JSON 失败: {}", e))?;

        let display = Self::parse_usage_response(&json)?;
        
        Ok((display, new_tokens))
    }

    /// 从 API 获取用量 (从 auth.json 读取 Token)
    pub async fn fetch_usage(refresh_token: Option<String>) -> Result<(UsageDisplay, Option<crate::oauth::TokenResponse>), String> {
        let (mut access_token, account_id) = Self::read_auth()?;
        let mut new_tokens: Option<crate::oauth::TokenResponse> = None;

        let client = reqwest::Client::new();
        let build_request = |at: &str, aid: &Option<String>| {
            let mut req = client
                .get("https://chatgpt.com/backend-api/wham/usage")
                .header("Authorization", format!("Bearer {}", at))
                .header("User-Agent", "CodexSwitcher/1.0")
                .header("Accept", "application/json")
                .timeout(std::time::Duration::from_secs(30));
            if let Some(id) = aid {
                req = req.header("ChatGPT-Account-Id", id);
            }
            req
        };

        let mut response = build_request(&access_token, &account_id).send().await
            .map_err(|e| format!("网络请求失败: {}", e))?;

        let mut status = response.status();
        
        // 如果 401 且有 refresh_token，尝试刷新
        if (status == 401 || status == 403) && refresh_token.is_some() {
            if let Some(ref rt) = refresh_token {
                if let Ok(token_res) = crate::oauth::refresh_access_token(rt).await {
                    access_token = token_res.access_token.clone();
                    new_tokens = Some(token_res);
                    
                    // 重试请求
                    response = build_request(&access_token, &account_id).send().await
                        .map_err(|e| format!("刷新后重试失败: {}", e))?;
                    status = response.status();
                }
            }
        }

        if status == 401 || status == 403 {
            return Err("认证失败，请重新登录 Codex".to_string());
        }

        if !status.is_success() {
            return Err(format!("API 返回错误: {}", status));
        }

        let text = response.text().await
            .map_err(|e| format!("读取响应失败: {}", e))?;

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("解析 JSON 失败: {}", e))?;

        let display = Self::parse_usage_response(&json)?;
        Ok((display, new_tokens))
    }

    /// 从 Value 解析用量数据
    fn parse_usage_response(json: &Value) -> Result<UsageDisplay, String> {
        let plan_type = json.get("plan_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // 解析 5 小时窗口
        let (five_hour_used, five_hour_reset, five_hour_reset_at) = Self::parse_window(
            json.get("rate_limit").and_then(|r| r.get("primary_window"))
        );

        // 解析周窗口
        let (weekly_used, weekly_reset, weekly_reset_at) = Self::parse_window(
            json.get("rate_limit").and_then(|r| r.get("secondary_window"))
        );

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
            .and_then(|v| Self::parse_number(v));

        // 解析允许状态
        let _is_allowed = json.get("rate_limit")
            .and_then(|r| r.get("allowed"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(UsageDisplay {
            plan_type,
            five_hour_used,
            five_hour_left: 100 - five_hour_used,
            five_hour_reset,
            five_hour_reset_at,
            weekly_used,
            weekly_left: 100 - weekly_used,
            weekly_reset,
            weekly_reset_at,
            credits_balance,
            has_credits: has_credits || unlimited,
            is_valid_for_cli: true, // 能走到这里说明 API 请求成功，Token 是有效的
        })
    }

    /// 解析窗口数据
    fn parse_window(window: Option<&Value>) -> (i32, String, Option<i64>) {
        let window = match window {
            Some(w) => w,
            None => return (0, "未知".to_string(), None),
        };

        let used_percent = window.get("used_percent")
            .and_then(|v| Self::parse_int(v))
            .unwrap_or(0);

        let reset_at = window.get("reset_at")
            .and_then(|v| Self::parse_int(v) as Option<i32>)
            .map(|v| v as i64)
            .or_else(|| window.get("reset_at").and_then(|v| v.as_i64()))
            .unwrap_or(0);

        let mut final_reset_ts = None;

        let reset_str = if reset_at > 0 {
            final_reset_ts = Some(reset_at);
            Self::format_reset(reset_at)
        } else {
            // 尝试使用 reset_after_seconds
            let reset_after = window.get("reset_after_seconds")
                .or_else(|| window.get("reset_after_sec"))
                .and_then(|v| Self::parse_int(v))
                .unwrap_or(0);
            if reset_after > 0 {
                use chrono::Utc;
                final_reset_ts = Some(Utc::now().timestamp() + reset_after as i64);
                Self::format_duration(reset_after as i64)
            } else {
                "未知".to_string()
            }
        };

        (used_percent, reset_str, final_reset_ts)
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

        let reset_time = Utc.timestamp_opt(reset_at, 0)
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
