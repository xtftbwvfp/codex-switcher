//! 临时邮箱抽象 + 多 provider 实现。
//! 仅供 OTP 自动登录使用，不影响其他模块。
//!
//! Providers:
//! - UsmailMyId（usmail.my.id 公共收件箱，按邮箱地址直查）
//! - SorryiosNet（sorryios.net，每账号一个 token，调 /back-api/code 直接拿最新码）

use regex::Regex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct OtpHit {
    pub code: String,
    pub from: String,
    pub subject: String,
}

/// 多 provider 派发器。各自封自己的轮询逻辑 + since 过滤。
pub enum MailboxProvider {
    Usmail(UsmailMyId),
    Sorryios(SorryiosNet),
    NissanSerena(NissanSerena),
}

impl MailboxProvider {
    pub async fn fetch_otp(&self, email: &str, deadline: Instant) -> Result<OtpHit, String> {
        match self {
            Self::Usmail(p) => {
                p.fetch_otp(email, deadline, &["openai.com", "noreply"])
                    .await
            }
            Self::Sorryios(p) => p.fetch_otp(email, deadline).await,
            Self::NissanSerena(p) => p.fetch_otp(email, deadline).await,
        }
    }
}

pub struct UsmailMyId {
    pub client: reqwest::Client,
    pub poll_every: Duration,
    /// 只接受这个时刻及以后到达的邮件（避免捡到上次 send-otp 的旧码）
    pub since_unix_ms: Option<i64>,
}

impl UsmailMyId {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            poll_every: Duration::from_secs(3),
            since_unix_ms: None,
        }
    }

    pub fn since_now(mut self) -> Self {
        // 减 30 秒兜底：usmail 与本机时钟可能有飘移
        self.since_unix_ms = Some(chrono::Utc::now().timestamp_millis() - 30_000);
        self
    }

    pub async fn fetch_otp(
        &self,
        email: &str,
        deadline: Instant,
        sender_filter: &[&str],
    ) -> Result<OtpHit, String> {
        // 优先：紧邻 code/验证码/verification/OpenAI 代码 等关键字的 6 位
        let semantic = Regex::new(
            r"(?i)(?:code\s+is|verification\s+code|one[- ]time\s+code|OpenAI\s+code|代码为|验证码[是为]?\s*[:：]?|代码\s+is|code\s+for)\s*[:：]?\s*(\d{6})",
        )
        .unwrap();
        // 兜底：单独成行的 6 位
        let standalone = Regex::new(r"(?m)^\s*(\d{6})\s*$").unwrap();
        // 最后兜底：任意 6 位（前后非数字）
        let any_six = Regex::new(r"(?:[^\d]|^)(\d{6})(?:[^\d]|$)").unwrap();
        let url = format!(
            "https://usmail.my.id/api/emails/{}",
            urlencoding::encode(email)
        );
        loop {
            if Instant::now() >= deadline {
                return Err("OTP 等待超时".into());
            }
            let resp = match self
                .client
                .get(&url)
                .header("accept", "application/json")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[Mailbox] 请求失败，重试: {e}");
                    tokio::time::sleep(self.poll_every).await;
                    continue;
                }
            };
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[Mailbox] JSON 解析失败: {e}");
                    tokio::time::sleep(self.poll_every).await;
                    continue;
                }
            };
            let emails = body.get("emails").and_then(|v| v.as_array());
            if let Some(arr) = emails {
                // usmail.my.id 返回顺序新→旧，第一项最新
                for m in arr.iter() {
                    let from = m
                        .get("from")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let subject = m
                        .get("subject")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let text = m.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let html = m.get("html").and_then(|v| v.as_str()).unwrap_or("");
                    if !sender_filter.is_empty()
                        && !sender_filter
                            .iter()
                            .any(|f| from.to_ascii_lowercase().contains(&f.to_ascii_lowercase()))
                    {
                        continue;
                    }
                    if let Some(since) = self.since_unix_ms {
                        // usmail.my.id: ISO-8601 字符串字段 "time"。其他兜底也尝试一下。
                        let mail_ms = [
                            "time",
                            "date",
                            "receivedAt",
                            "received_at",
                            "createdAt",
                            "created_at",
                        ]
                        .iter()
                        .find_map(|key| {
                            let v = m.get(*key)?;
                            if let Some(s) = v.as_str() {
                                chrono::DateTime::parse_from_rfc3339(s)
                                    .ok()
                                    .map(|d| d.timestamp_millis())
                                    .or_else(|| {
                                        chrono::DateTime::parse_from_rfc2822(s)
                                            .ok()
                                            .map(|d| d.timestamp_millis())
                                    })
                            } else {
                                v.as_i64()
                            }
                        });
                        if let Some(ms) = mail_ms {
                            if ms < since {
                                eprintln!(
                                    "[Mailbox] 跳过旧邮件 subject={} ms={} since={}",
                                    subject, ms, since
                                );
                                continue;
                            }
                        }
                    }
                    let haystack = format!("{subject}\n{text}\n{html}");
                    let mut hit_code: Option<String> = None;
                    for re in [&semantic, &standalone, &any_six] {
                        if let Some(c) = re.captures(&haystack) {
                            if let Some(g) = c.get(1) {
                                hit_code = Some(g.as_str().to_string());
                                break;
                            }
                        }
                    }
                    if let Some(code) = hit_code {
                        let mail_time = m
                            .get("time")
                            .or_else(|| m.get("date"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        eprintln!(
                            "[Mailbox] 选中: time={} from={} subject={} code={}",
                            mail_time, from, subject, code
                        );
                        return Ok(OtpHit {
                            code,
                            from,
                            subject,
                        });
                    }
                }
            }
            tokio::time::sleep(self.poll_every).await;
        }
    }
}

// ============================================================
// SorryiosNet: 私有 OTP 中转服务
// ============================================================
pub struct SorryiosNet {
    pub client: reqwest::Client,
    pub token: String,
    pub poll_every: Duration,
    pub since_unix_ms: Option<i64>,
    pub visitor_id: String,
}

impl SorryiosNet {
    pub fn new(client: reqwest::Client, token: String) -> Self {
        Self {
            client,
            token,
            // 服务端速率限制 20 秒/次（响应 code=602 "操作太频繁"），稍微留余量
            poll_every: Duration::from_secs(22),
            since_unix_ms: None,
            visitor_id: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub fn since_now(mut self) -> Self {
        self.since_unix_ms = Some(chrono::Utc::now().timestamp_millis() - 30_000);
        self
    }

    pub async fn fetch_otp(&self, email: &str, deadline: Instant) -> Result<OtpHit, String> {
        let referer = format!("https://www.sorryios.net/token/{}", self.token);
        // sorryios 的 email 字段大小写不敏感，但响应里返回 lowercase；这里规范化输入
        let email_norm = email.trim().to_ascii_lowercase();
        let body = json!({"email": email_norm, "token": self.token}).to_string();
        let device_info = json!({
            "timestamp": chrono::Utc::now().timestamp_millis(),
            "userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
            "timezone": "Asia/Shanghai"
        })
        .to_string();

        // 诊断累积：超时时一并暴露
        let mut polls: u32 = 0;
        let mut last_status: Option<u16> = None;
        let mut last_msg: Option<String> = None;
        let mut last_seen_code_time: Option<String> = None;
        let mut last_seen_code: Option<String> = None;

        loop {
            if Instant::now() >= deadline {
                let since_str = self
                    .since_unix_ms
                    .map(|ms| {
                        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_else(|| ms.to_string())
                    })
                    .unwrap_or_else(|| "<none>".into());
                return Err(format!(
                    "OTP 等待超时（sorryios.net）。polls={polls} since={since_str} last_status={last_status:?} last_msg={last_msg:?} last_seen_code_time={last_seen_code_time:?} last_seen_code={last_seen_code:?} email={email_norm}"
                ));
            }
            polls += 1;
            let resp = match self
                .client
                .post("https://www.sorryios.net/back-api/code")
                .header("origin", "https://www.sorryios.net")
                .header("referer", &referer)
                .header("content-type", "application/json")
                .header("accept", "*/*")
                .header("x-visitor-id", &self.visitor_id)
                .header("x-device-info", &device_info)
                // 必传：服务端 query_success_log 表 user_agent 字段 NOT NULL，缺它会 500
                .header(
                    "user-agent",
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
                )
                .body(body.clone())
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_msg = Some(format!("network: {e}"));
                    eprintln!("[Mailbox/sorryios] poll#{polls} 请求失败: {e}");
                    tokio::time::sleep(self.poll_every).await;
                    continue;
                }
            };
            let status = resp.status();
            last_status = Some(status.as_u16());
            let raw = resp.text().await.unwrap_or_default();
            eprintln!(
                "[Mailbox/sorryios] poll#{polls} status={} body={}",
                status,
                if raw.len() > 600 {
                    format!("{}…[{}b]", &raw[..600], raw.len())
                } else {
                    raw.clone()
                }
            );
            let v: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    last_msg = Some(format!("non-JSON status={status}: {e}"));
                    tokio::time::sleep(self.poll_every).await;
                    continue;
                }
            };
            // 服务端有时 success=false（还没收到验证码），继续轮
            let success = v.get("success").and_then(|x| x.as_bool()).unwrap_or(false);
            if !success {
                let msg = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("(no msg)");
                last_msg = Some(format!("success=false msg={msg}"));
                tokio::time::sleep(self.poll_every).await;
                continue;
            }
            let data = match v.get("data") {
                Some(d) => d,
                None => {
                    tokio::time::sleep(self.poll_every).await;
                    continue;
                }
            };
            let code = data
                .get("code")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let time_str = data.get("time").and_then(|x| x.as_str()).unwrap_or("");
            last_seen_code = Some(code.clone());
            last_seen_code_time = Some(time_str.to_string());
            // 时间过滤：跳过老码（time < since）
            if let Some(since) = self.since_unix_ms {
                let ms = chrono::DateTime::parse_from_rfc3339(time_str)
                    .ok()
                    .map(|d| d.timestamp_millis())
                    .or_else(|| {
                        chrono::DateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M:%S%.3f%z")
                            .ok()
                            .map(|d| d.timestamp_millis())
                    });
                if let Some(ms) = ms {
                    if ms < since {
                        eprintln!(
                            "[Mailbox/sorryios] 跳过老码 time={time_str} ms={ms} since={since}"
                        );
                        tokio::time::sleep(self.poll_every).await;
                        continue;
                    }
                } else {
                    // 解析不出来时间，按"算它新"处理（避免把好码错过）
                    eprintln!("[Mailbox/sorryios] 警告：time 字段无法解析: {time_str:?}");
                }
            }
            if code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) {
                eprintln!(
                    "[Mailbox/sorryios] 选中: time={time_str} email={email_norm} code={code}"
                );
                return Ok(OtpHit {
                    code,
                    from: "sorryios.net".to_string(),
                    subject: format!("OTP for {email_norm}"),
                });
            }
            tokio::time::sleep(self.poll_every).await;
        }
    }
}

// ============================================================
// NissanSerena: HTML-based OTP finder（覆盖 .my.id / .biz.id / .web.id 等域名）
// ============================================================
pub struct NissanSerena {
    pub client: reqwest::Client,
    pub poll_every: Duration,
    /// 第一次轮询拿到的 code（baseline）；之后看到不一样的 6 位才返回
    pub since_unix_ms: Option<i64>,
}

impl NissanSerena {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            poll_every: Duration::from_secs(5),
            since_unix_ms: None,
        }
    }

    pub fn since_now(mut self) -> Self {
        self.since_unix_ms = Some(chrono::Utc::now().timestamp_millis() - 60_000);
        self
    }

    /// 拉一次页面，返回 (top_code, top_time_string)
    async fn fetch_top(&self, email: &str) -> Result<Option<(String, String)>, String> {
        let url = format!(
            "https://nissanserena.my.id/otp/search?email={}",
            urlencoding::encode(email)
        );
        let resp = self
            .client
            .get(&url)
            .header("referer", "https://nissanserena.my.id/otp")
            .header(
                "user-agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
            )
            .header(
                "accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .send()
            .await
            .map_err(|e| format!("nissanserena GET 失败: {e}"))?;
        let html = resp
            .text()
            .await
            .map_err(|e| format!("nissanserena 读响应失败: {e}"))?;

        // 服务端按 newest→oldest 渲染。第一个 tracking-widest 块就是最新。
        let re_code = Regex::new(r#"tracking-widest">\s*(\d{6})"#).unwrap();
        let re_time = Regex::new(
            r"(\d{1,2})\s+(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+(\d{4}),\s*(\d{1,2}):(\d{2})",
        )
        .unwrap();
        let code = re_code
            .captures(&html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());
        let time = re_time
            .captures(&html)
            .map(|c| c.get(0).unwrap().as_str().to_string());
        Ok(code.map(|c| (c, time.unwrap_or_default())))
    }

    pub async fn fetch_otp(&self, email: &str, deadline: Instant) -> Result<OtpHit, String> {
        // 先 GET /otp 拿 session cookie（reqwest cookie_store 自动接管）
        let _ = self
            .client
            .get("https://nissanserena.my.id/otp")
            .send()
            .await;

        // baseline: 第一次拉到的 code，等到看见不同的 6 位（且时间 >= since 的话也合规）才返回
        let (mut baseline_code, mut baseline_time) = match self.fetch_top(email).await {
            Ok(Some((c, t))) => (Some(c), Some(t)),
            _ => (None, None),
        };
        eprintln!(
            "[Mailbox/nissanserena] baseline code={:?} time={:?}",
            baseline_code, baseline_time
        );

        let mut polls: u32 = 0;
        loop {
            if Instant::now() >= deadline {
                return Err(format!(
                    "OTP 等待超时（nissanserena.my.id）。polls={polls} baseline={:?} email={}",
                    baseline_code, email
                ));
            }
            tokio::time::sleep(self.poll_every).await;
            polls += 1;

            match self.fetch_top(email).await {
                Ok(Some((code, time))) => {
                    eprintln!("[Mailbox/nissanserena] poll#{polls} top code={code} time={time}");
                    let is_new = match &baseline_code {
                        Some(b) => *b != code,
                        None => true, // baseline 为空 → 任何码都算新
                    };
                    if is_new {
                        eprintln!(
                            "[Mailbox/nissanserena] 选中: time={time} email={email} code={code}"
                        );
                        return Ok(OtpHit {
                            code,
                            from: "nissanserena.my.id".to_string(),
                            subject: format!("OTP for {email}"),
                        });
                    }
                    // 同 code 继续等
                    if baseline_code.is_none() {
                        baseline_code = Some(code);
                        baseline_time = Some(time);
                    }
                }
                Ok(None) => {
                    eprintln!("[Mailbox/nissanserena] poll#{polls} 页面没找到 OTP");
                }
                Err(e) => {
                    eprintln!("[Mailbox/nissanserena] poll#{polls} 错误: {e}");
                }
            }
        }
    }
}
