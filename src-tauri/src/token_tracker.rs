//! Token 用量统计和费用计算
//!
//! 从代理转发的 SSE 流中提取 usage 数据，按模型价格计算费用。

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 模型定价表（美元 / 百万 token）
struct ModelPricing {
    input_per_million: f64,
    cached_input_per_million: f64,
    output_per_million: f64,
}

fn get_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    // OpenAI 2025 pricing (approximate)
    if m.contains("o3") {
        ModelPricing {
            input_per_million: 2.0,
            cached_input_per_million: 1.0,
            output_per_million: 8.0,
        }
    } else if m.contains("o4-mini") {
        ModelPricing {
            input_per_million: 1.10,
            cached_input_per_million: 0.275,
            output_per_million: 4.40,
        }
    } else if m.contains("codex-mini") {
        ModelPricing {
            input_per_million: 1.50,
            cached_input_per_million: 0.375,
            output_per_million: 6.00,
        }
    } else if m.contains("gpt-4.1") {
        ModelPricing {
            input_per_million: 2.00,
            cached_input_per_million: 0.50,
            output_per_million: 8.00,
        }
    } else if m.contains("gpt-4.1-mini") {
        ModelPricing {
            input_per_million: 0.40,
            cached_input_per_million: 0.10,
            output_per_million: 1.60,
        }
    } else {
        // 默认按中等价格
        ModelPricing {
            input_per_million: 2.00,
            cached_input_per_million: 0.50,
            output_per_million: 8.00,
        }
    }
}

/// 单次请求的 usage 数据
#[derive(Debug, Clone)]
pub struct RequestUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub model: String,
}

/// 累计统计数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub total_input_tokens: i64,
    pub total_cached_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub total_requests: u64,
    /// 按模型分拆的 token 数
    pub by_model: HashMap<String, ModelUsage>,
    /// 统计开始时间
    pub since: DateTime<Utc>,
    /// 上月同期对比数据
    pub last_month_cost: Option<f64>,
    pub last_month_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

impl Default for UsageStats {
    fn default() -> Self {
        Self {
            total_input_tokens: 0,
            total_cached_input_tokens: 0,
            total_output_tokens: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            total_requests: 0,
            by_model: HashMap::new(),
            since: Utc::now(),
            last_month_cost: None,
            last_month_tokens: None,
        }
    }
}

/// Token 统计追踪器
pub struct TokenTracker {
    stats: Mutex<UsageStats>,
}

impl TokenTracker {
    pub fn new() -> Arc<Self> {
        let mut stats = Self::load_from_disk().unwrap_or_default();
        // 每月重置
        let now = Utc::now();
        if stats.since.month() != now.month() || stats.since.year() != now.year() {
            let old = stats.clone();
            stats = UsageStats::default();
            stats.last_month_cost = Some(old.total_cost_usd);
            stats.last_month_tokens = Some(old.total_tokens);
        }
        Arc::new(Self {
            stats: Mutex::new(stats),
        })
    }

    /// 记录一次请求的 usage
    pub fn record(&self, usage: RequestUsage) {
        let pricing = get_pricing(&usage.model);

        let uncached_input = usage.input_tokens - usage.cached_input_tokens;
        let cost = (uncached_input as f64 * pricing.input_per_million
            + usage.cached_input_tokens as f64 * pricing.cached_input_per_million
            + usage.output_tokens as f64 * pricing.output_per_million)
            / 1_000_000.0;

        if let Ok(mut stats) = self.stats.lock() {
            stats.total_input_tokens += usage.input_tokens;
            stats.total_cached_input_tokens += usage.cached_input_tokens;
            stats.total_output_tokens += usage.output_tokens;
            stats.total_tokens += usage.total_tokens;
            stats.total_cost_usd += cost;
            stats.total_requests += 1;

            let model_entry = stats
                .by_model
                .entry(usage.model.clone())
                .or_insert(ModelUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                });
            model_entry.input_tokens += usage.input_tokens;
            model_entry.output_tokens += usage.output_tokens;
            model_entry.cost_usd += cost;

            // 持久化
            Self::save_to_disk(&stats);
        }
    }

    /// 获取当前统计快照
    pub fn get_stats(&self) -> UsageStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// 重置统计
    pub fn reset(&self) {
        if let Ok(mut stats) = self.stats.lock() {
            let old = stats.clone();
            *stats = UsageStats::default();
            stats.last_month_cost = Some(old.total_cost_usd);
            stats.last_month_tokens = Some(old.total_tokens);
            Self::save_to_disk(&stats);
        }
    }

    fn stats_path() -> PathBuf {
        dirs::home_dir()
            .expect("home dir")
            .join(".codex-switcher")
            .join("proxy-usage.json")
    }

    fn load_from_disk() -> Option<UsageStats> {
        let path = Self::stats_path();
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save_to_disk(stats: &UsageStats) {
        let path = Self::stats_path();
        if let Ok(json) = serde_json::to_string_pretty(stats) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// 从 SSE 流的累积数据中提取 usage 信息
/// 查找 `response.completed` 事件中的 `usage` 字段
pub fn extract_usage_from_sse(data: &[u8], request_model: &str) -> Option<RequestUsage> {
    let text = String::from_utf8_lossy(data);

    // SSE 格式：每个事件以 "data: " 开头
    // 查找包含 "response.completed" 的事件
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let json_str = line.trim_start_matches("data:").trim();
        if !json_str.contains("response.completed") {
            continue;
        }

        // 解析 JSON
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            let response = val.get("response")?;
            let usage = response.get("usage")?;

            let model = response
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or(request_model)
                .to_string();

            let input_tokens = usage
                .get("input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let cached_input = usage
                .get("input_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let output_tokens = usage
                .get("output_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let total_tokens = usage
                .get("total_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(input_tokens + output_tokens);

            if total_tokens > 0 {
                return Some(RequestUsage {
                    input_tokens,
                    cached_input_tokens: cached_input,
                    output_tokens,
                    total_tokens,
                    model,
                });
            }
        }
    }

    None
}
