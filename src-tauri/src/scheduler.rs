//! 后台调度器 - 账号状态同步
//!
//! 对齐 Codex 行为：不主动续期 Token，仅按间隔同步当前账号的 auth.json 状态。

use crate::account::AccountStore;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::time::Duration;

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
            let (enabled, interval_minutes) = {
                let store = store.lock().unwrap();
                (
                    store.settings.background_refresh,
                    store.settings.refresh_interval_minutes,
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

            // 仅同步当前账号：auth.json 是权威源，不对任何账号做主动 refresh_token 续期
            let mut synced_current = false;
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
                                    synced_current = true;
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

            if synced_current {
                let _ = app_handle.emit("accounts-updated", ());
            }

            tokio::time::sleep(Duration::from_secs(u64::from(interval_minutes) * 60)).await;
        }
    })
}
