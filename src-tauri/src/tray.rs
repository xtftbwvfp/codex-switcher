use crate::AppState;
use tauri::{
    image::Image,
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

/// 初始化系统托盘
pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let app_handle = app.clone();

    // 加载并按照 Antigravity 标准比例 (81.8%) 缩放原始 Squircle 图标
    let icon_bytes = include_bytes!("../icons/app-icon-squircle.png");
    let base_img =
        image::load_from_memory(icon_bytes).map_err(|e| format!("加载图标失败: {}", e))?;

    let target_size = 128;
    let content_size = 105; // 128 * 0.818
    let padding = (target_size - content_size) / 2;

    let scaled_content = base_img.resize(
        content_size,
        content_size,
        image::imageops::FilterType::Lanczos3,
    );
    let mut final_img = image::RgbaImage::new(target_size, target_size);

    image::imageops::overlay(
        &mut final_img,
        &scaled_content,
        padding as i64,
        padding as i64,
    );

    let (width, height) = final_img.dimensions();
    let icon = Image::new_owned(final_img.into_raw(), width, height);

    // 初始菜单
    let menu = Menu::new(app)?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .icon_as_template(false) // 关键：在 macOS 上强制禁用 Template 模式，以显示彩色图标
        .show_menu_on_left_click(false) // 关键：左键点击不弹出菜单，由我们自定义处理
        .on_menu_event(move |app: &AppHandle, event| {
            let app_handle = app.clone();
            match event.id.as_ref() {
                "next" => {
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<AppState>();
                        let _ = crate::switch_to_next_account_internal(state, app_handle.clone()).await;
                        update_tray_menu(&app_handle);
                        if let Some(win) = app_handle.get_webview_window("main") {
                            let _ = win.emit("accounts-updated", ());
                        }
                    });
                }
                "refresh" => {
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<AppState>();
                        let current_id = {
                            let store = state.store.lock().unwrap();
                            store.current.clone()
                        };
                        if let Some(id) = current_id {
                            let _ = crate::get_quota_by_id(state, app_handle.clone(), id).await;
                            update_tray_menu(&app_handle);
                        }
                    });
                }
                "show" => {
                    show_main_window(&app_handle);
                }
                "exit" => {
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray: &TrayIcon, event: TrayIconEvent| {
            // 核心：仅处理左键点击以显示窗口。
            // 不要处理右键点击，由系统自动处理菜单弹出，否则会造成菜单闪退。
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    // 初始生成菜单内容
    update_tray_menu(&app_handle);

    println!("✅ 系统托盘已启动 (已修复右键闪退与颜色问题)");
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show().unwrap_or(());
        let _ = window.unminimize().unwrap_or(());
        let _ = window.set_focus().unwrap_or(());
        #[cfg(target_os = "macos")]
        app.set_activation_policy(tauri::ActivationPolicy::Regular)
            .unwrap_or(());
    }
}

/// 重构后的菜单刷新逻辑：每次重建 Menu 对象以确保 UI 稳定性
pub fn update_tray_menu(app: &AppHandle) {
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let state = app_handle.state::<AppState>();
        let (email_text, gemini_text, claude_text) = {
            let store = state.store.lock().unwrap();
            let current_acc = store.current.as_ref().and_then(|id| store.accounts.get(id));

            if let Some(acc) = current_acc {
                let q = &acc.cached_quota;
                (
                    format!("当前: {}", acc.name),
                    q.as_ref()
                        .map(|v| format!("{}: {}%", v.five_hour_label, v.five_hour_left as i32))
                        .unwrap_or_else(|| "Gemini: -".into()),
                    q.as_ref()
                        .map(|v| format!("{}: {}%", v.weekly_label, v.weekly_left as i32))
                        .unwrap_or_else(|| "Claude: -".into()),
                )
            } else {
                (
                    "当前: 未登录".into(),
                    "Gemini: -".into(),
                    "Claude: -".into(),
                )
            }
        };

        // 2. 构建菜单项
        let info_i = MenuItemBuilder::with_id("info", email_text)
            .enabled(false)
            .build(&app_handle)
            .ok();
        let gemini_i = MenuItemBuilder::with_id("gemini", gemini_text)
            .enabled(false)
            .build(&app_handle)
            .ok();
        let claude_i = MenuItemBuilder::with_id("claude", claude_text)
            .enabled(false)
            .build(&app_handle)
            .ok();

        let sep1 = PredefinedMenuItem::separator(&app_handle).ok();

        // 获取下个账号预览
        let next_preview = crate::predict_next_account_internal(state);
        let next_label = match next_preview {
            Some((name, quota)) => format!("切换下一个账号 (下个: {} {}%)", name, quota),
            None => "切换下一个账号 (无可用备选)".into(),
        };

        let next_acc = MenuItemBuilder::with_id("next", next_label)
            .build(&app_handle)
            .ok();
        let refresh_acc = MenuItemBuilder::with_id("refresh", "刷新当前账号额度")
            .build(&app_handle)
            .ok();

        let sep2 = PredefinedMenuItem::separator(&app_handle).ok();
        let show_win = MenuItemBuilder::with_id("show", "显示主窗口")
            .build(&app_handle)
            .ok();

        let sep3 = PredefinedMenuItem::separator(&app_handle).ok();
        let exit = MenuItemBuilder::with_id("exit", "退出应用 (Exit)")
            .build(&app_handle)
            .ok();

        // 3. 构建菜单数组
        let mut items = Vec::new();
        if let Some(i) = info_i {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = gemini_i {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = claude_i {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }

        if let Some(s) = sep1 {
            items.push(Box::new(s) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = next_acc {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = refresh_acc {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }

        if let Some(s) = sep2 {
            items.push(Box::new(s) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = show_win {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }

        if let Some(s) = sep3 {
            items.push(Box::new(s) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }
        if let Some(i) = exit {
            items.push(Box::new(i) as Box<dyn tauri::menu::IsMenuItem<_>>);
        }

        let item_refs: Vec<&dyn tauri::menu::IsMenuItem<_>> =
            items.iter().map(|b| b.as_ref()).collect();

        // 4. 应用新菜单并维持属性
        if let Ok(new_menu) = Menu::with_items(&app_handle, &item_refs) {
            if let Some(tray) = app_handle.tray_by_id("main") {
                let _ = tray.set_menu(Some(new_menu));
                // 重要：在某些平台上，set_menu 可能会重置点击行为，这里再次声明。
                let _ = tray.set_show_menu_on_left_click(false);
            }
        }
    });
}
