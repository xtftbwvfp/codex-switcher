//! Codex Switcher - 账号管理模块
//! 
//! 处理多个 Codex 账号的存储、切换和管理
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

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
    #[serde(default = "default_true")]
    pub background_refresh: bool,
    
    /// 刷新间隔（分钟）
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_minutes: u32,

    /// 主题设置: "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_primary_ide() -> String {
    "Windsurf".to_string()
}

fn default_refresh_interval() -> u32 {
    30
}

fn default_theme() -> String {
    "light".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_reload_ide: false,
            primary_ide: default_primary_ide(),
            use_pkill_restart: false,
            background_refresh: true,
            refresh_interval_minutes: default_refresh_interval(),
            theme: default_theme(),
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
    pub five_hour_reset_at: Option<i64>,
    pub weekly_left: f64,
    pub weekly_reset: String,
    pub weekly_reset_at: Option<i64>,
    pub plan_type: String,
    #[serde(default = "default_true")]
    pub is_valid_for_cli: bool,
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool { true }

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
        if path.exists() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    /// 保存账号存储
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        
        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败: {}", e))?;
        }
        
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("序列化失败: {}", e))?;
        
        fs::write(&path, content)
            .map_err(|e| format!("写入文件失败: {}", e))?;
        
        Ok(())
    }

    /// 读取当前 Codex auth.json
    pub fn read_codex_auth() -> Result<serde_json::Value, String> {
        let path = Self::codex_auth_path();
        if !path.exists() {
            return Err("未找到 Codex auth.json，请先登录 Codex".to_string());
        }
        
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("读取 auth.json 失败: {}", e))?;
        
        serde_json::from_str(&content)
            .map_err(|e| format!("解析 auth.json 失败: {}", e))
    }

    /// 写入 Codex auth.json
    pub fn write_codex_auth(auth: &serde_json::Value) -> Result<(), String> {
        let path = Self::codex_auth_path();
        println!("写入 auth.json 到路径: {:?}", path);
        
        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败: {}", e))?;
        }
        
        let content = serde_json::to_string_pretty(auth)
            .map_err(|e| format!("序列化失败: {}", e))?;
        
        // 原子写入：先写临时文件，再重命名
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, content)
            .map_err(|e| format!("写入临时文件失败: {}", e))?;
            
        fs::rename(&tmp_path, &path)
            .map_err(|e| format!("重命名文件失败 (Atomic Write): {}", e))?;
        
        Ok(())
    }

    /// 添加新账号
    pub fn add_account(&mut self, name: String, auth_json: serde_json::Value, notes: Option<String>) -> Account {
        let id = uuid::Uuid::new_v4().to_string();
        let account = Account {
            id: id.clone(),
            name,
            auth_json,
            refresh_token: None, // 默认 None，由调用者后续按需设置
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
        let account = self.accounts.get_mut(id)
            .ok_or_else(|| format!("账号不存在: {}", id))?;
        
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
    pub fn update_account(&mut self, id: &str, name: Option<String>, notes: Option<String>) -> Result<(), String> {
        let account = self.accounts.get_mut(id)
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
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("导出失败: {}", e))
    }

    /// 导入配置
    pub fn import(json: &str) -> Result<Self, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("导入失败: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
