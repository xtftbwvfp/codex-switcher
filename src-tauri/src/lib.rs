//! Codex Switcher - Tauri 主入口
//!
//! 暴露所有 Tauri 命令供前端调用

mod account;
mod ide_control;
mod oauth;
mod oauth_server;
mod refresh_lock;
mod scheduler;
mod tray;
mod usage;

use account::{Account, AccountStore};
use chrono::Utc;
use refresh_lock::RefreshLockManager;
use tauri::{Manager, State};
use usage::{UsageDisplay, UsageFetcher};

const QUARANTINE_FIX_TICKET_TTL_SECS: i64 = 120;

#[derive(Clone, Debug)]
struct QuarantineFixTicket {
    value: String,
    expires_at: chrono::DateTime<Utc>,
}

fn allow_local_refresh_for_quota(is_current: bool) -> bool {
    // 手工刷新时仅允许“非当前账号”走本地 refresh_token 续期。
    // 当前账号交由 Codex 官方流程维护，避免双端并发续期导致 token reused。
    !is_current
}

fn detect_sync_conflict_for_current(
    account: &Account,
    disk_auth: &serde_json::Value,
) -> Option<String> {
    // 身份不一致时不应提示“Token 冲突”，避免误判
    if !AccountStore::auth_identity_matches(&account.auth_json, disk_auth) {
        return None;
    }

    let official_rt = AccountStore::extract_refresh_token(disk_auth);
    let local_rt = AccountStore::extract_refresh_token(&account.auth_json);

    // 如果官方 Token 存在且与本地不同（通常是更新了），则视为冲突
    if official_rt.is_some() && official_rt != local_rt {
        return Some(account.name.clone());
    }

    None
}

/// 应用状态
pub struct AppState {
    pub store: std::sync::Arc<std::sync::Mutex<AccountStore>>,
    pub scheduler: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub refresh_locks: RefreshLockManager,
    quarantine_fix_ticket: std::sync::Mutex<Option<QuarantineFixTicket>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            store: std::sync::Arc::new(std::sync::Mutex::new(AccountStore::load())),
            scheduler: std::sync::Mutex::new(None),
            refresh_locks: RefreshLockManager::default(),
            quarantine_fix_ticket: std::sync::Mutex::new(None),
        }
    }

    fn issue_quarantine_fix_ticket(&self) -> Result<String, String> {
        let ticket = uuid::Uuid::new_v4().to_string();
        let expires_at = Utc::now() + chrono::Duration::seconds(QUARANTINE_FIX_TICKET_TTL_SECS);
        let mut slot = self
            .quarantine_fix_ticket
            .lock()
            .map_err(|e| e.to_string())?;
        *slot = Some(QuarantineFixTicket {
            value: ticket.clone(),
            expires_at,
        });
        Ok(ticket)
    }

    fn consume_quarantine_fix_ticket(&self, provided_ticket: &str) -> Result<(), String> {
        let mut slot = self
            .quarantine_fix_ticket
            .lock()
            .map_err(|e| e.to_string())?;
        let now = Utc::now();
        match slot.take() {
            Some(stored) if stored.expires_at < now => {
                Err("安全确认已过期，请重新点击修复".to_string())
            }
            Some(stored) if stored.value != provided_ticket => {
                Err("安全确认无效，请重新点击修复".to_string())
            }
            Some(_) => Ok(()),
            None => Err("缺少安全确认，请重新点击修复".to_string()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
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
fn update_settings(
    state: State<AppState>,
    app: tauri::AppHandle,
    settings: account::AppSettings,
) -> Result<(), String> {
    let previous = {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        let previous = store.settings.background_refresh;
        store.settings = settings.clone();
        store.save()?;
        previous
    };

    let mut scheduler_handle = state.scheduler.lock().map_err(|e| e.to_string())?;

    match (previous, settings.background_refresh) {
        (false, true) => {
            if scheduler_handle.is_none() {
                let handle = scheduler::start(state.store.clone(), app);
                *scheduler_handle = Some(handle);
            }
        }
        (true, false) => {
            if let Some(handle) = scheduler_handle.take() {
                handle.abort();
            }
        }
        _ => {}
    }

    Ok(())
}

/// 从当前 Codex 登录状态导入账号
#[tauri::command]
fn import_current_account(
    state: State<AppState>,
    name: String,
    notes: Option<String>,
) -> Result<Account, String> {
    let auth_json = AccountStore::read_codex_auth()?;
    if AccountStore::extract_refresh_token(&auth_json).is_none() {
        return Err("当前 auth.json 缺少 refresh_token，无法自动续期，请重新登录".to_string());
    }

    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    let account = store.add_account(name, auth_json, notes);
    store.save()?;

    Ok(account)
}

// is_token_expired removed: align with Codex last_refresh-based refresh

/// 检查当前 IDE 中的账号是否有未同步的 Token 更新
#[tauri::command]
fn check_sync_conflict(state: State<AppState>) -> Result<Option<String>, String> {
    let auth_json = match AccountStore::read_codex_auth() {
        Ok(a) => a,
        Err(_) => return Ok(None), // 如果由于文件不存在等原因读取失败，视为无冲突
    };

    let store = state.store.lock().map_err(|e| e.to_string())?;

    // 检查这个 auth.json 是否属于我们当前的活跃账号，且内容是否有变
    if let Some(current_id) = &store.current {
        if let Some(account) = store.accounts.get(current_id) {
            if let Some(name) = detect_sync_conflict_for_current(account, &auth_json) {
                return Ok(Some(name));
            }
        }
    }

    Ok(None)
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
fn update_account(
    state: State<AppState>,
    id: String,
    name: Option<String>,
    notes: Option<String>,
) -> Result<(), String> {
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
    let missing = new_store.accounts_missing_refresh_token();
    if !missing.is_empty() {
        return Err(format!(
            "以下账号缺少 refresh_token，无法自动续期，请重新登录后再导入: {}",
            missing.join(", ")
        ));
    }
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    *store = new_store;
    store.save()?;
    Ok(())
}

/// 完成 OAuth 登录并保存账号
#[tauri::command]
async fn finalize_oauth_login(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<Account, String> {
    let token_res = oauth_server::complete_oauth_login(code).await?;
    if token_res.refresh_token.is_none() {
        return Err("OAuth 未返回 refresh_token，无法自动续期".to_string());
    }

    let user_info = token_res
        .id_token
        .as_ref()
        .and_then(|id_t| oauth::parse_user_info(id_t))
        .ok_or("无法从授权响应中解析用户信息 (Missing ID Token)")?;

    let mut store = state.store.lock().map_err(|e| e.to_string())?;

    // 计算过期时间
    let expires_at = token_res
        .expires_in
        .map(|secs| (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());

    let auth_json = serde_json::json!({
        "tokens": {
            "access_token": token_res.access_token,
            "refresh_token": token_res.refresh_token,
            "id_token": token_res.id_token,
            "account_id": user_info.account_id,
            "expires_at": expires_at
        },
        "last_refresh": chrono::Utc::now().to_rfc3339()
    });

    let mut account = store.add_account(
        user_info.email,
        auth_json,
        Some("OpenAI OAuth 登录".to_string()),
    );

    account.refresh_token = token_res.refresh_token.clone();
    if let Some(acc) = store.accounts.get_mut(&account.id) {
        acc.refresh_token = token_res.refresh_token;
    }

    store.save()?;
    Ok(account)
}

/// 切换到指定账号（异步版本，不做本地 Token 续期）
#[tauri::command]
async fn switch_account(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    // 0. 切换前仅同步“当前激活账号”与官方 auth.json，避免全表匹配导致串号
    if let Ok(current_auth) = AccountStore::read_codex_auth() {
        if let Ok(mut store) = state.store.lock() {
            if let Some(current_id) = store.current.clone() {
                if store.sync_account_from_auth_json(&current_id, current_auth) {
                    if let Err(e) = store.save() {
                        eprintln!("[Sync] 保存当前账号失败: {}", e);
                    }
                }
            }
        }
    }

    // 1. 获取目标账号的校验凭据
    let (target_id, access_token, refresh_token, account_id) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let account = store
            .accounts
            .get(&id)
            .ok_or_else(|| format!("账号 {} 不存在", id))?;

        let access_token = account
            .auth_json
            .get("tokens")
            .and_then(|t| t.get("access_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or("账号缺少 access_token")?;

        let refresh_token = account.refresh_token.clone();

        let account_id = account
            .auth_json
            .get("account_id")
            .or_else(|| {
                account
                    .auth_json
                    .get("tokens")
                    .and_then(|t| t.get("account_id"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        (account.id.clone(), access_token, refresh_token, account_id)
    };

    // 2. 预检（非阻断）：仅尝试读取配额缓存，不触发本地 refresh_token 刷新。
    // 失败不阻断切换，交由 Codex 在实际请求中按需维护 token 生命周期。
    println!(
        "[Switch] 预检目标账号配额（不触发本地 refresh）: {}",
        target_id
    );
    match usage::UsageFetcher::fetch_usage_direct(access_token, account_id, refresh_token, false)
        .await
    {
        Ok((usage, _)) => {
            let mut store = state.store.lock().map_err(|e| e.to_string())?;
            if let Some(account) = store.accounts.get_mut(&target_id) {
                account.cached_quota = Some(account::CachedQuota {
                    five_hour_left: usage.five_hour_left as f64,
                    five_hour_reset: usage.five_hour_reset.clone(),
                    five_hour_label: usage.five_hour_label.clone(),
                    weekly_left: usage.weekly_left as f64,
                    weekly_reset: usage.weekly_reset.clone(),
                    weekly_label: usage.weekly_label.clone(),
                    plan_type: usage.plan_type.clone(),
                    is_valid_for_cli: usage.is_valid_for_cli,
                    updated_at: chrono::Utc::now(),
                });
                let _ = store.save();
            }
        }
        Err(e) => {
            println!("[Switch] 预检配额失败（忽略，不阻断切换）: {}", e);
        }
    }

    // 3. 执行切换 (写入 auth.json)
    println!("[Switch] 执行切换...");
    if !state
        .refresh_locks
        .acquire(&target_id, tokio::time::Duration::from_secs(5))
        .await
    {
        return Err("该账号正在被其他流程刷新，请稍后重试".to_string());
    }
    let switch_result: Result<(), String> = {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        match store.switch_to(&target_id) {
            Ok(()) => store.save(),
            Err(e) => Err(e),
        }
    };
    state.refresh_locks.release(&target_id).await;
    switch_result?;

    println!("[Switch] 切换完成！");
    Ok(())
}

/// 将当前 Codex auth.json 强制同步到指定账号
#[tauri::command]
fn sync_current_auth_to_account(state: State<AppState>, id: String) -> Result<(), String> {
    let auth_json = AccountStore::read_codex_auth()?;
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    if store.sync_account_from_auth_json(&id, auth_json) {
        store.save()?;
        return Ok(());
    }
    Err("同步失败：账号不存在或 User ID 不匹配".to_string())
}

/// 检查 Codex 是否已登录
#[tauri::command]
fn check_codex_login() -> Result<bool, String> {
    Ok(AccountStore::codex_auth_path().exists())
}

/// 获取指定账号的用量信息（不切换账号）
#[tauri::command]
async fn get_quota_by_id(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<UsageDisplay, String> {
    // 当前激活账号：先按 ~/.codex/auth.json 做身份校验与同步，再继续走 API 查询配额
    let is_current = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.current.as_deref() == Some(id.as_str())
    };

    if is_current {
        let official_auth = AccountStore::read_codex_auth()?;
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        let local_auth = store
            .accounts
            .get(&id)
            .ok_or_else(|| format!("账号 {} 不存在", id))?
            .auth_json
            .clone();

        if !AccountStore::auth_identity_matches(&local_auth, &official_auth) {
            return Err(
                "当前激活账号与 ~/.codex/auth.json 身份不匹配，已拒绝覆盖，请先在 Codex 中切回同一账号".to_string(),
            );
        }

        if local_auth != official_auth {
            println!(
                "[Quota] 当前激活账号 {}：检测到官方 auth.json 变更，按权威源同步。",
                id
            );
            if store.sync_account_from_auth_json(&id, official_auth) {
                store.save()?;
            }
        } else {
            println!("[Quota] 当前激活账号 {}：已与官方 auth.json 保持一致。", id);
        }
    }

    // 1. 从 Store 获取该账号的 Token
    let (access_token, account_id, refresh_token) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let account = store
            .accounts
            .get(&id)
            .ok_or_else(|| format!("账号 {} 不存在", id))?;

        // 从 auth_json 中提取 access_token 和 account_id
        let tokens = account
            .auth_json
            .get("tokens")
            .ok_or("账号数据缺少 tokens 字段")?;

        let at = tokens
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or("账号数据缺少 access_token")?
            .to_string();

        let aid = tokens
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let rt = account
            .refresh_token
            .clone()
            .or_else(|| AccountStore::extract_refresh_token(&account.auth_json));

        (at, aid, rt)
    };

    // 2. 直接使用该账号的 Token 获取用量
    let allow_local_refresh = allow_local_refresh_for_quota(is_current);
    let (usage, new_tokens) = UsageFetcher::fetch_usage_direct(
        access_token,
        account_id,
        refresh_token,
        allow_local_refresh,
    )
    .await?;

    // 3. 如果有新 Token，更新该账号的数据
    if let Some(tokens) = new_tokens {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        if let Some(account) = store.accounts.get_mut(&id) {
            // 更新 auth_json 中的 Token 信息
            if let Some(obj) = account.auth_json.as_object_mut() {
                if let Some(tokens_obj) = obj.get_mut("tokens").and_then(|v| v.as_object_mut()) {
                    tokens_obj.insert(
                        "access_token".to_string(),
                        serde_json::json!(tokens.access_token),
                    );

                    if let Some(rt) = &tokens.refresh_token {
                        tokens_obj.insert("refresh_token".to_string(), serde_json::json!(rt));
                    } else if let Some(rt) = account.refresh_token.as_deref() {
                        if tokens_obj.get("refresh_token").is_none() {
                            tokens_obj.insert("refresh_token".to_string(), serde_json::json!(rt));
                        }
                    }

                    if let Some(it) = &tokens.id_token {
                        tokens_obj.insert("id_token".to_string(), serde_json::json!(it));
                    }

                    if let Some(expires_in) = tokens.expires_in {
                        let expires_at = (chrono::Utc::now()
                            + chrono::Duration::seconds(expires_in as i64))
                        .to_rfc3339();
                        tokens_obj.insert("expires_at".to_string(), serde_json::json!(expires_at));
                    }
                }
            }

            // 更新 refresh_token 字段
            if let Some(rt) = tokens.refresh_token {
                account.refresh_token = Some(rt);
            }
            if let Some(obj) = account.auth_json.as_object_mut() {
                obj.insert(
                    "last_refresh".to_string(),
                    serde_json::json!(Utc::now().to_rfc3339()),
                );
            }

            // 更新配额缓存
            account.cached_quota = Some(account::CachedQuota {
                five_hour_left: usage.five_hour_left as f64,
                five_hour_reset: usage.five_hour_reset.clone(),
                five_hour_label: usage.five_hour_label.clone(),
                weekly_left: usage.weekly_left as f64,
                weekly_reset: usage.weekly_reset.clone(),
                weekly_label: usage.weekly_label.clone(),
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
                five_hour_label: usage.five_hour_label.clone(),
                weekly_left: usage.weekly_left as f64,
                weekly_reset: usage.weekly_reset.clone(),
                weekly_label: usage.weekly_label.clone(),
                plan_type: usage.plan_type.clone(),
                is_valid_for_cli: usage.is_valid_for_cli,
                updated_at: Utc::now(),
            });
        }
        store.save()?;
    }

    Ok(usage)
}

/// 修复 Codex App 的隔离属性 (需要 sudo 权限)
#[tauri::command]
fn request_quarantine_fix_ticket(state: State<AppState>) -> Result<String, String> {
    state.issue_quarantine_fix_ticket()
}

/// 修复 Codex App 的隔离属性 (需要 sudo 权限)
#[tauri::command]
async fn fix_codex_quarantine(
    state: tauri::State<'_, AppState>,
    ticket: String,
) -> Result<(), String> {
    state.consume_quarantine_fix_ticket(&ticket)?;
    ide_control::remove_quarantine()
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

            // 启动后台调度器（仅在设置开启时）
            let state = app.state::<AppState>();
            let should_start = state
                .store
                .lock()
                .map(|store| store.settings.background_refresh)
                .unwrap_or(false);
            if should_start {
                let handle = scheduler::start(state.store.clone(), app.handle().clone());
                let mut scheduler_handle = state.scheduler.lock().unwrap();
                *scheduler_handle = Some(handle);
            } else {
                println!("[Scheduler] 后台刷新未开启，跳过启动");
            }

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
                    app.set_activation_policy(tauri::ActivationPolicy::Accessory)
                        .unwrap_or(());
                }
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_accounts,
            get_current_account_id,
            import_current_account,
            switch_account,
            sync_current_auth_to_account,
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
            check_sync_conflict,
            request_quarantine_fix_ticket,
            fix_codex_quarantine,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_auth(account_id: &str, refresh_token: &str) -> serde_json::Value {
        serde_json::json!({
            "tokens": {
                "account_id": account_id,
                "refresh_token": refresh_token
            }
        })
    }

    fn test_account(name: &str, account_id: &str, refresh_token: &str) -> Account {
        let auth_json = test_auth(account_id, refresh_token);
        Account {
            id: "acc-1".to_string(),
            name: name.to_string(),
            auth_json: auth_json.clone(),
            refresh_token: AccountStore::extract_refresh_token(&auth_json),
            created_at: Utc::now(),
            last_used: None,
            notes: None,
            cached_quota: None,
        }
    }

    #[test]
    fn quota_refresh_allows_only_non_current_account_local_token_refresh() {
        assert!(!allow_local_refresh_for_quota(true));
        assert!(allow_local_refresh_for_quota(false));
    }

    #[test]
    fn sync_conflict_is_ignored_when_identity_mismatch() {
        let current = test_account("current", "acct-local", "rt-local");
        let disk_auth = test_auth("acct-disk", "rt-new");

        assert_eq!(detect_sync_conflict_for_current(&current, &disk_auth), None);
    }

    #[test]
    fn sync_conflict_is_reported_when_identity_matches_and_refresh_token_changed() {
        let current = test_account("current", "acct-1", "rt-local");
        let disk_auth = test_auth("acct-1", "rt-new");

        assert_eq!(
            detect_sync_conflict_for_current(&current, &disk_auth),
            Some("current".to_string())
        );
    }

    #[test]
    fn quarantine_fix_ticket_can_only_be_used_once() {
        let state = AppState::new();
        let ticket = state.issue_quarantine_fix_ticket().unwrap();

        assert!(state.consume_quarantine_fix_ticket(&ticket).is_ok());
        assert!(state.consume_quarantine_fix_ticket(&ticket).is_err());
    }

    #[test]
    fn quarantine_fix_ticket_rejects_mismatch() {
        let state = AppState::new();
        let _ticket = state.issue_quarantine_fix_ticket().unwrap();

        assert!(state.consume_quarantine_fix_ticket("wrong-ticket").is_err());
    }

    #[test]
    fn quarantine_fix_ticket_rejects_expired_ticket() {
        let state = AppState::new();
        {
            let mut slot = state.quarantine_fix_ticket.lock().unwrap();
            *slot = Some(QuarantineFixTicket {
                value: "expired".to_string(),
                expires_at: Utc::now() - chrono::Duration::seconds(1),
            });
        }

        let err = state
            .consume_quarantine_fix_ticket("expired")
            .expect_err("expired ticket should be rejected");
        assert!(err.contains("过期"));
    }
}
