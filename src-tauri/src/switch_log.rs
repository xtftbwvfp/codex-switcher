//! 切号日志模块
//!
//! 记录每次切号的时间、来源、目标、原因，持久化到 JSONL 文件。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwitchReason {
    Manual,
    Http429,
    QuotaThreshold,
    WebSocketPrecheck,
    WebSocketRateLimit,
    BannedDetected,
    AutoQuotaRefresh,
    BackgroundKeepalive,
}

impl std::fmt::Display for SwitchReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwitchReason::Manual => write!(f, "手动切号"),
            SwitchReason::Http429 => write!(f, "429 限额"),
            SwitchReason::QuotaThreshold => write!(f, "阈值预防"),
            SwitchReason::WebSocketPrecheck => write!(f, "WS 预检"),
            SwitchReason::WebSocketRateLimit => write!(f, "WS 限额"),
            SwitchReason::BannedDetected => write!(f, "封号检测"),
            SwitchReason::AutoQuotaRefresh => write!(f, "自动刷新"),
            SwitchReason::BackgroundKeepalive => write!(f, "后台保活"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchEvent {
    pub timestamp: DateTime<Utc>,
    pub from_account: Option<String>,
    pub to_account: String,
    pub reason: SwitchReason,
    pub from_quota_5h: Option<f64>,
    pub to_quota_5h: Option<f64>,
}

/// 切号日志管理器
pub struct SwitchLogger {
    events: Mutex<Vec<SwitchEvent>>,
}

impl SwitchLogger {
    pub fn new() -> std::sync::Arc<Self> {
        let mut events = Self::load_from_disk();
        // 清理 30 天前的记录
        let cutoff = Utc::now() - chrono::Duration::days(30);
        events.retain(|e| e.timestamp > cutoff);

        std::sync::Arc::new(Self {
            events: Mutex::new(events),
        })
    }

    /// 记录一次切号事件
    pub fn log_switch(
        &self,
        from_account: Option<String>,
        to_account: String,
        reason: SwitchReason,
        from_quota_5h: Option<f64>,
        to_quota_5h: Option<f64>,
    ) {
        let event = SwitchEvent {
            timestamp: Utc::now(),
            from_account,
            to_account,
            reason,
            from_quota_5h,
            to_quota_5h,
        };

        println!(
            "[SwitchLog] {} → {} ({})",
            event.from_account.as_deref().unwrap_or("无"),
            event.to_account,
            event.reason
        );

        if let Ok(mut events) = self.events.lock() {
            events.push(event);
            Self::save_to_disk(&events);
        }
    }

    /// 获取最近 N 天的切号记录
    pub fn get_history(&self, days: u32) -> Vec<SwitchEvent> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        self.events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|e| e.timestamp > cutoff)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 统计摘要
    pub fn get_stats(&self) -> SwitchStats {
        let events = self.events.lock().map(|e| e.clone()).unwrap_or_default();
        let now = Utc::now();
        let today_start = now - chrono::Duration::hours(24);
        let week_start = now - chrono::Duration::days(7);

        let today_count = events.iter().filter(|e| e.timestamp > today_start).count();
        let week_count = events.iter().filter(|e| e.timestamp > week_start).count();
        let total_count = events.len();

        // 按原因统计
        let mut by_reason = std::collections::HashMap::new();
        for e in &events {
            let key = format!("{}", e.reason);
            *by_reason.entry(key).or_insert(0u64) += 1;
        }

        // 按目标账号统计
        let mut by_account = std::collections::HashMap::new();
        for e in &events {
            *by_account.entry(e.to_account.clone()).or_insert(0u64) += 1;
        }

        SwitchStats {
            today_count: today_count as u64,
            week_count: week_count as u64,
            total_count: total_count as u64,
            by_reason,
            by_account,
        }
    }

    fn log_path() -> PathBuf {
        dirs::home_dir()
            .expect("home dir")
            .join(".codex-switcher")
            .join("switch-history.jsonl")
    }

    fn load_from_disk() -> Vec<SwitchEvent> {
        let path = Self::log_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .filter_map(|line| serde_json::from_str::<SwitchEvent>(line).ok())
            .collect()
    }

    fn save_to_disk(events: &[SwitchEvent]) {
        let path = Self::log_path();
        let content: Vec<String> = events
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect();
        let _ = std::fs::write(path, content.join("\n") + "\n");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchStats {
    pub today_count: u64,
    pub week_count: u64,
    pub total_count: u64,
    pub by_reason: std::collections::HashMap<String, u64>,
    pub by_account: std::collections::HashMap<String, u64>,
}
