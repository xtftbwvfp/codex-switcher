//! 后台调度器 - 账号状态同步
//!
//! 策略：
//! - 当前账号：仅按官方 auth.json 回流同步，不主动 refresh
//! - 非活跃账号：由 Switcher 独占执行保活 refresh，并原子回写账号库

use crate::account::AccountStore;
use crate::oauth;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::time::Duration;

#[derive(Debug, Clone)]
struct RefreshTarget {
    id: String,
    name: String,
    refresh_token: String,
}

#[derive(Serialize, Clone)]
struct RefreshFailedPayload {
    account_name: String,
    reason: String,
}

fn is_reused_or_revoked_error(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    lower.contains("refresh_token_reused")
        || lower.contains("refresh_token_invalidated")
        || lower.contains("refresh_token_expired")
        || lower.contains("deactivated")
        || lower.contains("unauthorized")
        || lower.contains("invalid_grant")
}

fn is_logged_out_error(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    lower.contains("logged out") || lower.contains("signed in to another account")
}

/// 启动后台状态同步调度器
pub fn start(
    store: Arc<Mutex<AccountStore>>,
    app_handle: tauri::AppHandle,
) -> tauri::async_runtime::JoinHandle<()> {
    // 使用 Tauri 的 async runtime 而不是直接 tokio::spawn
    // 因为在 setup() 中调用时 Tokio runtime 可能尚未完全初始化
    tauri::async_runtime::spawn(async move {
        println!("✅ 后台调度器已启动");

        loop {
            let (enabled, interval_minutes, inactive_refresh_days) = {
                let store = store.lock().unwrap();
                (
                    store.settings.background_refresh,
                    store.settings.refresh_interval_minutes,
                    store.settings.inactive_refresh_days,
                )
            };

            if !enabled {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }

            println!("[Scheduler] 开始后台同步检查...");

            let interval_minutes = if interval_minutes == 0 {
                30
            } else {
                interval_minutes
            };

            let mut store_changed = false;
            let mut has_failure_event = false;

            // 1) 同步当前账号（权威源：~/.codex/auth.json）
            if let Ok(official_auth) = AccountStore::read_codex_auth() {
                let mut store = store.lock().unwrap();
                if let Some(current_id) = store.current.clone() {
                    let local_auth = store.accounts.get(&current_id).map(|a| a.auth_json.clone());

                    if let Some(local_auth) = local_auth {
                        if AccountStore::auth_identity_matches(&local_auth, &official_auth) {
                            if local_auth != official_auth {
                                println!(
                                    "[Scheduler] 当前账号 {} 检测到官方 auth.json 变动，按权威源同步。",
                                    current_id
                                );
                                if store.sync_account_from_auth_json(&current_id, official_auth) {
                                    let _ = store.save();
                                    store_changed = true;
                                    println!("[Scheduler] ✅ 当前账号反向同步成功");
                                }
                            } else {
                                println!(
                                    "[Scheduler] 当前账号 {} 与官方 auth.json 一致。",
                                    current_id
                                );
                            }
                        } else {
                            println!(
                                "[Scheduler] 当前账号 {} 与官方 auth.json 身份不匹配，跳过同步。",
                                current_id
                            );
                        }
                    }
                }
            }

            // 2) 收集应由 Switcher 独占保活的非活跃账号
            let targets: Vec<RefreshTarget> = {
                let store = store.lock().unwrap();
                let current = store.current.as_deref();
                store
                    .accounts
                    .values()
                    .filter(|account| current != Some(account.id.as_str()))
                    .filter(|account| {
                        AccountStore::should_refresh_inactive_account(
                            account,
                            inactive_refresh_days,
                        )
                    })
                    .filter_map(|account| {
                        let rt = account
                            .refresh_token
                            .clone()
                            .or_else(|| AccountStore::extract_refresh_token(&account.auth_json))?;
                        Some(RefreshTarget {
                            id: account.id.clone(),
                            name: account.name.clone(),
                            refresh_token: rt,
                        })
                    })
                    .collect()
            };

            // 3) 对非活跃账号执行独占保活刷新
            for target in targets {
                println!("[Scheduler] 非活跃账号 {} 尝试保活刷新", target.name);

                match oauth::refresh_access_token(&target.refresh_token).await {
                    Ok(tokens) => {
                        let mut store = store.lock().unwrap();
                        if store.current.as_deref() == Some(target.id.as_str()) {
                            // 账号已变为当前，交给官方路径维护
                            continue;
                        }
                        if let Some(account) = store.accounts.get_mut(&target.id) {
                            if !account.keepalive.inactive_refresh_enabled {
                                continue;
                            }
                            AccountStore::apply_refreshed_tokens(
                                account,
                                tokens.access_token,
                                tokens.refresh_token,
                                tokens.id_token,
                                tokens.expires_in,
                            );
                        }
                        store.mark_keepalive_attempt_success(&target.id);
                        let _ = store.save();
                        store_changed = true;
                        println!("[Scheduler] ✅ 非活跃账号 {} 保活刷新成功", target.name);

                        // 记录后台保活系统日志
                        use tauri::Manager;
                        if let Some(logger) =
                            app_handle.try_state::<Arc<crate::switch_log::SwitchLogger>>()
                        {
                            logger.inner().log_switch(
                                None,
                                target.name.clone(),
                                crate::switch_log::SwitchReason::BackgroundKeepalive,
                                None,
                                None,
                            );
                        }
                    }
                    Err(err) => {
                        let reason = err;
                        let mut store = store.lock().unwrap();
                        store.mark_keepalive_attempt_failed(&target.id, reason.clone());
                        if is_reused_or_revoked_error(&reason) || is_logged_out_error(&reason) {
                            // 风险保护：检测到 reused/revoked 后，自动停用该账号的非活跃保活，避免重复消耗。
                            let _ = store.set_inactive_refresh_enabled(&target.id, false);
                            if let Some(account) = store.accounts.get_mut(&target.id) {
                                if is_logged_out_error(&reason) {
                                    account.is_logged_out = true;
                                } else {
                                    account.is_token_invalid = true;
                                }
                            }
                        }
                        let _ = store.save();
                        has_failure_event = true;
                        println!(
                            "[Scheduler] ❌ 非活跃账号 {} 保活刷新失败: {}",
                            target.name, reason
                        );

                        let _ = app_handle.emit(
                            "token-refresh-failed",
                            RefreshFailedPayload {
                                account_name: target.name,
                                reason,
                            },
                        );
                    }
                }
            }

            if store_changed || has_failure_event {
                let _ = app_handle.emit("accounts-updated", ());
            }

            tokio::time::sleep(Duration::from_secs(u64::from(interval_minutes) * 60)).await;
        }
    })
}
