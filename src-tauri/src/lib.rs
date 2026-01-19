//! Codex Switcher - Tauri 主入口
//! 
//! 暴露所有 Tauri 命令供前端调用

mod ide_control;
mod account;
mod usage;
mod oauth;
mod oauth_server;
mod tray;
mod scheduler;


use std::sync::Mutex;
use account::{Account, AccountStore};
use usage::{UsageFetcher, UsageDisplay};
use tauri::{State, Manager};
use chrono::Utc;
use base64::Engine;

/// 应用状态
pub struct AppState {
    store: Mutex<AccountStore>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(AccountStore::load()),
        }
    }
}

/// 获取所有账号
#[tauri::command]
fn get_accounts(state: State<AppState>) -> Result<Vec<Account>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(store.list_accounts().into_iter().cloned().collect())
}

/// 获取当前激活的账号 ID
#[tauri::command]
fn get_current_account_id(state: State<AppState>) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(store.current.clone())
}

/// 获取全局设置
#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<account::AppSettings, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(store.settings.clone())
}

/// 更新全局设置
#[tauri::command]
fn update_settings(state: State<AppState>, settings: account::AppSettings) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    store.settings = settings;
    store.save()?;
    Ok(())
}

/// 从当前 Codex 登录状态导入账号
#[tauri::command]
fn import_current_account(state: State<AppState>, name: String, notes: Option<String>) -> Result<Account, String> {
    let auth_json = AccountStore::read_codex_auth()?;
    
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    let account = store.add_account(name, auth_json, notes);
    store.save()?;
    
    Ok(account)
}

/// 检查 JWT Access Token 是否过期
fn is_token_expired(token: &str) -> bool {
    // JWT 格式: header.payload.signature
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return true; // 无效格式，视为过期
    }
    
    // 解码 payload (Base64URL)
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(p) => p,
        Err(_) => return true,
    };
    
    let json: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(j) => j,
        Err(_) => return true,
    };
    
    // 获取 exp 字段
    let exp = match json.get("exp").and_then(|v| v.as_i64()) {
        Some(e) => e,
        None => return true,
    };
    
    // 提前 5 分钟视为过期，避免边界问题
    let now = chrono::Utc::now().timestamp();
    exp < (now + 300)
}

/// 切换到指定账号（异步版本，自动刷新 Token）
#[tauri::command]
async fn switch_account(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    // 1. 获取账号数据
    let (auth_json, refresh_token) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let account = store.accounts.get(&id)
            .ok_or_else(|| format!("账号 {} 不存在", id))?;
        
        (account.auth_json.clone(), account.refresh_token.clone())
    };

    // 2. 判断是否需要刷新 Token
    let mut final_auth_json = auth_json.clone();
    let mut final_refresh_token = refresh_token.clone();
    
    let access_token = auth_json.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|at| at.as_str())
        .unwrap_or("");
        
    let should_refresh = is_token_expired(access_token);

    if should_refresh && refresh_token.is_some() {
        let rt = refresh_token.as_ref().unwrap();
        println!("Token 已过期或即将过期，正在尝试刷新...");
        
        match oauth::refresh_access_token(rt).await {
            Ok(token_res) => {
                println!("Token 刷新成功！");
                
                if let Some(obj) = final_auth_json.as_object_mut() {
                    if let Some(tokens_obj) = obj.get_mut("tokens").and_then(|v| v.as_object_mut()) {
                        tokens_obj.insert("access_token".to_string(), serde_json::json!(token_res.access_token));
                        if let Some(new_rt) = &token_res.refresh_token {
                            tokens_obj.insert("refresh_token".to_string(), serde_json::json!(new_rt));
                            final_refresh_token = Some(new_rt.clone());
                        }
                        if let Some(new_id) = &token_res.id_token {
                            tokens_obj.insert("id_token".to_string(), serde_json::json!(new_id));
                        }
                        
                        let seconds = token_res.expires_in.unwrap_or(3600);
                        let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(seconds as i64)).to_rfc3339();
                        tokens_obj.insert("expires_at".to_string(), serde_json::json!(expires_at));
                    }
                }
            }
            Err(e) => {
                eprintln!("Token 刷新失败: {}，将使用旧 Token 尝试", e);
            }
        }
    } else {
        println!("Token 仍在有效期内，直接使用。");
    }

    // 无论是否刷新，都更新 last_refresh 以满足 CLI 校验
    if let Some(obj) = final_auth_json.as_object_mut() {
        obj.insert("last_refresh".to_string(), serde_json::json!(chrono::Utc::now().to_rfc3339()));
    }

    // 3. 统一写入 auth.json 并更新 Store
    {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        store.current = Some(id.clone());
        
        if let Some(account) = store.accounts.get_mut(&id) {
            account.last_used = Some(chrono::Utc::now());
            account.auth_json = final_auth_json.clone();
            account.refresh_token = final_refresh_token;
        }
        
        AccountStore::write_codex_auth(&final_auth_json)?;
        store.save()?;
    }
    
    println!("账号切换成功: auth.json 已更新");
    Ok(())
}

/// 删除账号
#[tauri::command]
fn delete_account(state: State<AppState>, id: String) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete_account(&id)?;
    store.save()?;
    Ok(())
}

/// 更新账号信息
#[tauri::command]
fn update_account(state: State<AppState>, id: String, name: Option<String>, notes: Option<String>) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    store.update_account(&id, name, notes)?;
    store.save()?;
    Ok(())
}

/// 导出所有账号配置
#[tauri::command]
fn export_accounts(state: State<AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.export()
}

/// 导入账号配置
#[tauri::command]
fn import_accounts(state: State<AppState>, json: String) -> Result<(), String> {
    let new_store = AccountStore::import(&json)?;
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    *store = new_store;
    store.save()?;
    Ok(())
}

/// 完成 OAuth 登录并保存账号
#[tauri::command]
async fn finalize_oauth_login(state: tauri::State<'_, AppState>, code: String) -> Result<Account, String> {
    let token_res = oauth_server::complete_oauth_login(code).await?;
    
    let user_info = token_res.id_token.as_ref()
        .and_then(|id_t| oauth::parse_user_info(id_t))
        .ok_or("无法从授权响应中解析用户信息 (Missing ID Token)")?;
    
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    
    // 计算过期时间
    let expires_at = token_res.expires_in.map(|secs| {
        (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
    });
    
    let auth_json = serde_json::json!({
        "tokens": {
            "access_token": token_res.access_token,
            "refresh_token": token_res.refresh_token,
            "id_token": token_res.id_token,
            "account_id": user_info.account_id,
            "expires_at": expires_at
        }
    });

    let mut account = store.add_account(
        user_info.email,
        auth_json,
        Some("OpenAI OAuth 登录".to_string())
    );
    
    account.refresh_token = token_res.refresh_token.clone();
    if let Some(acc) = store.accounts.get_mut(&account.id) {
        acc.refresh_token = token_res.refresh_token;
    }
    
    store.save()?;
    Ok(account)
}

/// 检查 Codex 是否已登录
#[tauri::command]
fn check_codex_login() -> Result<bool, String> {
    Ok(AccountStore::codex_auth_path().exists())
}

/// 获取指定账号的用量信息（不切换账号）
#[tauri::command]
async fn get_quota_by_id(state: tauri::State<'_, AppState>, id: String) -> Result<UsageDisplay, String> {
    // 1. 从 Store 获取该账号的 Token
    let (access_token, account_id, refresh_token) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let account = store.accounts.get(&id)
            .ok_or_else(|| format!("账号 {} 不存在", id))?;
        
        // 从 auth_json 中提取 access_token 和 account_id
        let tokens = account.auth_json.get("tokens")
            .ok_or("账号数据缺少 tokens 字段")?;
        
        let at = tokens.get("access_token")
            .and_then(|v| v.as_str())
            .ok_or("账号数据缺少 access_token")?
            .to_string();
        
        let aid = tokens.get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        let rt = account.refresh_token.clone();
        
        (at, aid, rt)
    };

    // 2. 直接使用该账号的 Token 获取用量
    let (usage, new_tokens) = UsageFetcher::fetch_usage_direct(access_token, account_id, refresh_token).await?;
    
    // 3. 如果有新 Token，更新该账号的数据
    if let Some(tokens) = new_tokens {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        if let Some(account) = store.accounts.get_mut(&id) {
            
            // 更新 auth_json 中的 access_token
            if let Some(obj) = account.auth_json.as_object_mut() {
                if let Some(tokens_obj) = obj.get_mut("tokens").and_then(|v| v.as_object_mut()) {
                    tokens_obj.insert("access_token".to_string(), serde_json::json!(tokens.access_token));
                    if let Some(rt) = &tokens.refresh_token {
                        tokens_obj.insert("refresh_token".to_string(), serde_json::json!(rt));
                    }
                }
            }

            // 更新 refresh_token 字段
            if let Some(rt) = tokens.refresh_token {
                account.refresh_token = Some(rt);
            }

            // 更新配额缓存
            account.cached_quota = Some(account::CachedQuota {
                five_hour_left: usage.five_hour_left as f64,
                five_hour_reset: usage.five_hour_reset.clone(),
                weekly_left: usage.weekly_left as f64,
                weekly_reset: usage.weekly_reset.clone(),
                plan_type: usage.plan_type.clone(),
                is_valid_for_cli: usage.is_valid_for_cli,
                updated_at: Utc::now(),
            });
        }
        store.save()?;
    } else {
        // 即使没有新 Token，也更新配额缓存
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        if let Some(account) = store.accounts.get_mut(&id) {
            account.cached_quota = Some(account::CachedQuota {
                five_hour_left: usage.five_hour_left as f64,
                five_hour_reset: usage.five_hour_reset.clone(),
                weekly_left: usage.weekly_left as f64,
                weekly_reset: usage.weekly_reset.clone(),
                plan_type: usage.plan_type.clone(),
                is_valid_for_cli: usage.is_valid_for_cli,
                updated_at: Utc::now(),
            });
        }
        store.save()?;
    }

    Ok(usage)
}

/// 重载 IDE 窗口
#[tauri::command]
async fn reload_ide_windows(use_window_reload: bool) -> Result<Vec<String>, String> {
    let ides = ide_control::detect_running_ides();
    let mut reloaded = Vec::new();

    for ide in &ides {
        if let Err(e) = ide_control::reload_ide(ide, use_window_reload) {
            println!("重载 {} 失败: {}", ide, e);
        } else {
            reloaded.push(ide.clone());
        }
    }

    Ok(reloaded)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .setup(|app| {
            // 初始化系统托盘
            if let Err(e) = tray::init(app.handle()) {
                eprintln!("初始化托盘失败: {:?}", e);
            }
            
            // 启动后台调度器
            let store = app.state::<AppState>().store.lock().unwrap().clone();
            let store_arc = std::sync::Arc::new(std::sync::Mutex::new(store));
            scheduler::start(store_arc);
            
            Ok(())
        })
        .on_window_event(|window, event| {
            // 拦截关闭事件，改为隐藏窗口并从 Dock 隐藏
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                // macOS: 隐藏 Dock 图标，变成纯后台托盘应用
                #[cfg(target_os = "macos")]
                {
                    let app = window.app_handle();
                    app.set_activation_policy(tauri::ActivationPolicy::Accessory).unwrap_or(());
                }
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_accounts,
            get_current_account_id,
            import_current_account,
            switch_account,
            delete_account,
            update_account,
            export_accounts,
            import_accounts,
            check_codex_login,
            get_quota_by_id,
            oauth_server::start_oauth_login,
            finalize_oauth_login,
            reload_ide_windows,
            get_settings,
            update_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
