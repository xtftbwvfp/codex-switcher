//! Codex Switcher - 账号管理模块
//!
//! 处理多个 Codex 账号的存储、切换和管理
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 应用全局设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// 是否在切换账号后自动重载 IDE
    #[serde(default)]
    pub auto_reload_ide: bool,

    /// 主力 IDE: "Windsurf" | "Antigravity" | "Cursor" | "VSCode"
    #[serde(default = "default_primary_ide")]
    pub primary_ide: String,

    /// 是否使用杀进程方式重启（Windsurf 推荐）
    #[serde(default)]
    pub use_pkill_restart: bool,

    /// 后台自动刷新 Token
    #[serde(default = "default_false")]
    pub background_refresh: bool,

    /// 刷新间隔（分钟）
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_minutes: u32,
}

fn default_primary_ide() -> String {
    "Windsurf".to_string()
}

fn default_refresh_interval() -> u32 {
    30
}

fn default_false() -> bool {
    false
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_reload_ide: false,
            primary_ide: default_primary_ide(),
            use_pkill_restart: false,
            background_refresh: false,
            refresh_interval_minutes: default_refresh_interval(),
        }
    }
}

/// 单个账号信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// 唯一标识符
    pub id: String,
    /// 账号名称（用户自定义）
    pub name: String,
    /// auth.json 内容
    pub auth_json: serde_json::Value,
    /// OpenAI refresh_token (用于生成新的 auth_json)
    pub refresh_token: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 上次使用时间
    pub last_used: Option<DateTime<Utc>>,
    /// 备注
    pub notes: Option<String>,
    /// 缓存的配额信息
    #[serde(default)]
    pub cached_quota: Option<CachedQuota>,
}

/// 缓存的配额信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedQuota {
    pub five_hour_left: f64,
    pub five_hour_reset: String,
    #[serde(default = "default_five_hour_label")]
    pub five_hour_label: String,
    pub weekly_left: f64,
    pub weekly_reset: String,
    #[serde(default = "default_weekly_label")]
    pub weekly_label: String,
    pub plan_type: String,
    #[serde(default = "default_true")]
    pub is_valid_for_cli: bool,
    pub updated_at: DateTime<Utc>,
}

fn default_five_hour_label() -> String {
    "5H 限额".to_string()
}

fn default_weekly_label() -> String {
    "周限额".to_string()
}

fn default_true() -> bool {
    true
}

/// 账号存储结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountStore {
    /// 所有账号
    pub accounts: HashMap<String, Account>,
    /// 当前激活的账号 ID
    pub current: Option<String>,
    /// 版本号（用于迁移）
    pub version: u32,
    /// 全局设置
    #[serde(default)]
    pub settings: AppSettings,
}

#[cfg(unix)]
fn ensure_private_file_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms).map_err(|e| format!("设置文件权限失败: {}", e))
}

#[cfg(not(unix))]
fn ensure_private_file_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn ensure_private_dir_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o700);
    fs::set_permissions(path, perms).map_err(|e| format!("设置目录权限失败: {}", e))
}

#[cfg(not(unix))]
fn ensure_private_dir_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn write_text_secure(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("写入文件失败: {}", e))?;
    ensure_private_file_permissions(path)?;
    Ok(())
}

impl AccountStore {
    /// 配置文件路径
    pub fn config_path() -> PathBuf {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".codex-switcher")
            .join("accounts.json")
    }

    /// Codex auth.json 路径
    pub fn codex_auth_path() -> PathBuf {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".codex")
            .join("auth.json")
    }

    /// 加载账号存储
    pub fn load() -> Self {
        let path = Self::config_path();
        let mut store = if path.exists() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        };

        if store.backfill_refresh_tokens() {
            let _ = store.save();
        }

        store
    }

    /// 保存账号存储
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();

        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
            ensure_private_dir_permissions(parent)?;
        }

        let content =
            serde_json::to_string_pretty(self).map_err(|e| format!("序列化失败: {}", e))?;

        write_text_secure(&path, &content)?;

        Ok(())
    }

    /// 读取当前 Codex auth.json
    pub fn read_codex_auth() -> Result<serde_json::Value, String> {
        let path = Self::codex_auth_path();
        if !path.exists() {
            return Err("未找到 Codex auth.json，请先登录 Codex".to_string());
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("读取 auth.json 失败: {}", e))?;

        serde_json::from_str(&content).map_err(|e| format!("解析 auth.json 失败: {}", e))
    }

    /// 写入 Codex auth.json
    pub fn write_codex_auth(auth: &serde_json::Value) -> Result<(), String> {
        let path = Self::codex_auth_path();
        println!("写入 auth.json 到路径: {:?}", path);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
            ensure_private_dir_permissions(parent)?;
        }

        let content =
            serde_json::to_string_pretty(auth).map_err(|e| format!("序列化失败: {}", e))?;

        // 原子写入：先写临时文件，再重命名
        let tmp_path = path.with_extension("tmp");
        write_text_secure(&tmp_path, &content).map_err(|e| format!("写入临时文件失败: {}", e))?;

        fs::rename(&tmp_path, &path)
            .map_err(|e| format!("重命名文件失败 (Atomic Write): {}", e))?;
        ensure_private_file_permissions(&path)?;

        Ok(())
    }

    /// 添加新账号
    pub fn add_account(
        &mut self,
        name: String,
        auth_json: serde_json::Value,
        notes: Option<String>,
    ) -> Account {
        let id = uuid::Uuid::new_v4().to_string();
        let refresh_token = Self::extract_refresh_token(&auth_json);
        let account = Account {
            id: id.clone(),
            name,
            auth_json,
            refresh_token, // 从 auth_json 尝试提取
            created_at: Utc::now(),
            last_used: None,
            notes,
            cached_quota: None,
        };

        self.accounts.insert(id.clone(), account.clone());

        // 如果是第一个账号，设为当前
        if self.current.is_none() {
            self.current = Some(id);
        }

        account
    }

    /// 切换到指定账号
    pub fn switch_to(&mut self, id: &str) -> Result<(), String> {
        let account = self
            .accounts
            .get_mut(id)
            .ok_or_else(|| format!("账号不存在: {}", id))?;

        // 对齐 Codex：切换时不主动刷新 refresh_token，直接写入目标账号 auth.json。
        // 后续 token 生命周期由 Codex 自己在真实请求中按需维护。

        // 更新最后使用时间
        account.last_used = Some(Utc::now());

        // 写入 auth.json
        println!("正在切换账号: {}", id);
        Self::write_codex_auth(&account.auth_json)?;
        println!("账号切换成功: auth.json 已更新");

        // 更新当前账号
        self.current = Some(id.to_string());

        Ok(())
    }

    /// 删除账号
    pub fn delete_account(&mut self, id: &str) -> Result<(), String> {
        if !self.accounts.contains_key(id) {
            return Err(format!("账号不存在: {}", id));
        }

        self.accounts.remove(id);

        // 如果删除的是当前账号，清空 current
        if self.current.as_deref() == Some(id) {
            self.current = self.accounts.keys().next().cloned();
        }

        Ok(())
    }

    /// 更新账号信息
    pub fn update_account(
        &mut self,
        id: &str,
        name: Option<String>,
        notes: Option<String>,
    ) -> Result<(), String> {
        let account = self
            .accounts
            .get_mut(id)
            .ok_or_else(|| format!("账号不存在: {}", id))?;

        if let Some(n) = name {
            account.name = n;
        }
        if notes.is_some() {
            account.notes = notes;
        }

        Ok(())
    }

    /// 获取所有账号列表
    pub fn list_accounts(&self) -> Vec<&Account> {
        let mut accounts: Vec<_> = self.accounts.values().collect();
        accounts.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        accounts
    }

    /// 导出配置
    pub fn export(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("导出失败: {}", e))
    }

    /// 导入配置
    pub fn import(json: &str) -> Result<Self, String> {
        let mut store: Self = serde_json::from_str(json).map_err(|e| format!("导入失败: {}", e))?;
        store.backfill_refresh_tokens();
        Ok(store)
    }

    /// 从 auth_json 中提取 refresh_token（兼容 tokens.refresh_token 或根级 refresh_token）
    pub fn extract_refresh_token(auth_json: &Value) -> Option<String> {
        auth_json
            .get("tokens")
            .and_then(|t| t.get("refresh_token"))
            .or_else(|| auth_json.get("refresh_token"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    /// 从 auth_json 中提取 account_id
    pub fn extract_account_id(auth_json: &Value) -> Option<String> {
        auth_json
            .get("tokens")
            .and_then(|t| t.get("account_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    /// 账号身份是否一致（优先 account_id，其次 openai user id）
    pub fn auth_identity_matches(local_auth: &Value, external_auth: &Value) -> bool {
        let local_account_id = Self::extract_account_id(local_auth);
        let external_account_id = Self::extract_account_id(external_auth);
        if let (Some(local), Some(external)) =
            (local_account_id.as_deref(), external_account_id.as_deref())
        {
            return local == external;
        }

        let local_uid = Self::extract_openai_user_id(local_auth);
        let external_uid = Self::extract_openai_user_id(external_auth);
        if let (Some(local), Some(external)) = (local_uid.as_deref(), external_uid.as_deref()) {
            return local == external;
        }

        false
    }

    fn extract_jwt_claims_from_auth(auth_json: &Value, token_key: &str) -> Option<Value> {
        let token = auth_json
            .get("tokens")
            .and_then(|t| t.get(token_key))
            .or_else(|| auth_json.get(token_key))
            .and_then(|v| v.as_str())?;

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        use base64::Engine;
        let payload_part = parts[1];
        let mut padded = payload_part.to_string();
        while !padded.len().is_multiple_of(4) {
            padded.push('=');
        }

        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_part)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&padded))
            .ok()?;
        let json_str = String::from_utf8(decoded).ok()?;
        serde_json::from_str(&json_str).ok()
    }

    /// 从 auth_json 中提取邮箱（优先 id_token claims）
    pub fn extract_email(auth_json: &Value) -> Option<String> {
        let claims = Self::extract_jwt_claims_from_auth(auth_json, "id_token")
            .or_else(|| Self::extract_jwt_claims_from_auth(auth_json, "access_token"))?;

        claims
            .get("email")
            .and_then(|v| v.as_str())
            .or_else(|| {
                claims
                    .get("https://api.openai.com/profile")
                    .and_then(|v| v.get("email"))
                    .and_then(|v| v.as_str())
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    /// 从 auth_json 中提取 last_refresh（RFC3339 或时间戳）
    pub fn extract_last_refresh(auth_json: &Value) -> Option<DateTime<Utc>> {
        let raw = auth_json.get("last_refresh")?;
        if let Some(s) = raw.as_str() {
            return chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok();
        }
        if let Some(ts) = raw.as_i64() {
            let secs = if ts > 1_000_000_000_000 {
                ts / 1000
            } else {
                ts
            };
            return chrono::DateTime::<Utc>::from_timestamp(secs, 0);
        }
        None
    }

    /// 是否需要按间隔触发本地刷新（已停用，统一交由 Codex 按需维护）
    pub fn needs_refresh_by_interval(_auth_json: &Value) -> bool {
        false
    }

    /// 为缺失 refresh_token 的账号做一次回填
    fn backfill_refresh_tokens(&mut self) -> bool {
        let mut changed = false;
        for account in self.accounts.values_mut() {
            if account
                .refresh_token
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(false)
            {
                account.refresh_token = None;
                changed = true;
            }
            if account.refresh_token.is_none() {
                if let Some(rt) = Self::extract_refresh_token(&account.auth_json) {
                    account.refresh_token = Some(rt);
                    changed = true;
                }
            }
        }
        changed
    }

    /// 列出缺失 refresh_token 的账号（用于导入校验）
    pub fn accounts_missing_refresh_token(&self) -> Vec<String> {
        self.accounts
            .values()
            .filter(|account| account.refresh_token.is_none())
            .map(|account| account.name.clone())
            .collect()
    }

    pub fn accounts_missing_last_refresh(&self) -> Vec<String> {
        self.accounts
            .values()
            .filter(|account| Self::extract_last_refresh(&account.auth_json).is_none())
            .map(|account| account.name.clone())
            .collect()
    }

    /// 使用提供的 auth.json 同步指定账号
    /// 返回是否发生了更新
    pub fn sync_account_from_auth_json(&mut self, id: &str, auth_json: Value) -> bool {
        if let Some(account) = self.accounts.get_mut(id) {
            return Self::sync_account_from_auth_json_inner(account, auth_json);
        }
        false
    }

    fn sync_account_from_auth_json_inner(account: &mut Account, auth_json: Value) -> bool {
        // 安全检查：必须满足“身份一致（account_id/uid）”
        let local_account_id = Self::extract_account_id(&account.auth_json);
        let external_account_id = Self::extract_account_id(&auth_json);
        let local_uid = Self::extract_openai_user_id(&account.auth_json);
        let external_uid = Self::extract_openai_user_id(&auth_json);

        if !Self::auth_identity_matches(&account.auth_json, &auth_json) {
            eprintln!(
                "拒绝同步：身份不匹配 (外部 account_id: {:?}, 本地 account_id: {:?}, 外部 uid: {:?}, 本地 uid: {:?})",
                external_account_id, local_account_id, external_uid, local_uid
            );
            return false;
        }

        let local_name = account.name.trim().to_lowercase();
        let external_email = Self::extract_email(&auth_json).map(|s| s.to_lowercase());
        if local_name.contains('@') {
            if let Some(email) = external_email {
                if email != local_name {
                    eprintln!(
                        "拒绝同步：账号名与 token 邮箱不一致 (name: {:?}, token email: {:?})",
                        account.name, email
                    );
                    return false;
                }
            }
        }

        Self::sync_account_auth(account, auth_json);
        true
    }

    fn sync_account_auth(account: &mut Account, mut auth_json: Value) {
        if auth_json.get("last_refresh").is_none() {
            if let Some(existing) = account.auth_json.get("last_refresh") {
                if let Some(obj) = auth_json.as_object_mut() {
                    obj.insert("last_refresh".to_string(), existing.clone());
                }
            }
        }

        let new_rt = Self::extract_refresh_token(&auth_json);
        let fallback_rt = new_rt
            .clone()
            .or_else(|| account.refresh_token.clone())
            .or_else(|| Self::extract_refresh_token(&account.auth_json));

        if let Some(rt) = fallback_rt.as_deref() {
            if let Some(obj) = auth_json.as_object_mut() {
                if let Some(tokens_obj) = obj.get_mut("tokens").and_then(|v| v.as_object_mut()) {
                    if tokens_obj.get("refresh_token").is_none() {
                        tokens_obj.insert("refresh_token".to_string(), serde_json::json!(rt));
                    }
                }
            }
        }

        if let Some(rt) = new_rt {
            account.refresh_token = Some(rt);
        }

        account.auth_json = auth_json;
    }

    /// 从 auth_json 中提取 OpenAI User ID (从 access_token JWT)
    pub fn extract_openai_user_id(auth_json: &Value) -> Option<String> {
        let claims = Self::extract_jwt_claims_from_auth(auth_json, "access_token")?;

        // 尝试获取 user_id
        claims
            .get("https://api.openai.com/auth/user_id")
            .and_then(|v| v.as_str())
            .or_else(|| claims.get("sub").and_then(|v| v.as_str())) // sub 通常也是 ID
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_id_token(email: &str, account_id: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
            r#"{{"email":"{}","https://api.openai.com/auth":{{"chatgpt_account_id":"{}"}}}}"#,
            email, account_id
        ));
        format!("{header}.{payload}.sig")
    }

    fn auth_with_identity(email: &str, account_id: &str, refresh_token: &str) -> Value {
        serde_json::json!({
            "tokens": {
                "account_id": account_id,
                "refresh_token": refresh_token,
                "id_token": make_id_token(email, account_id),
                "access_token": "at.test.token"
            }
        })
    }

    #[test]
    fn test_add_account() {
        let mut store = AccountStore::default();
        let account = store.add_account(
            "测试账号".to_string(),
            serde_json::json!({"token": "test"}),
            None,
        );

        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.current, Some(account.id));
    }

    #[test]
    fn sync_rejects_when_email_mismatch_even_if_identity_matches() {
        let mut store = AccountStore::default();
        let local = auth_with_identity("hasbfarthoucapi@mail.com", "acct-1", "rt-a");
        let external = auth_with_identity("xtftbwvfp2025@outlook.com", "acct-1", "rt-b");
        let account = store.add_account("hasbfarthoucapi@mail.com".to_string(), local, None);

        let changed = store.sync_account_from_auth_json(&account.id, external);
        assert!(!changed, "email mismatch must reject sync");
    }

    #[test]
    fn sync_rejects_when_only_refresh_token_matches_but_identity_differs() {
        let mut store = AccountStore::default();
        let local = auth_with_identity("a@example.com", "acct-local", "rt-same");
        let external = auth_with_identity("a@example.com", "acct-other", "rt-same");
        let account = store.add_account("a@example.com".to_string(), local, None);

        let changed = store.sync_account_from_auth_json(&account.id, external);
        assert!(!changed, "refresh token equality must not be enough");
    }
}
