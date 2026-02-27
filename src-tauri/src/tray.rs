use tauri::{
    image::Image,
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

/// 初始化系统托盘
pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // 加载托盘图标 - 使用 image crate 解码 PNG 到 RGBA
    let icon_bytes = include_bytes!("../icons/32x32.png");
    let img = image::load_from_memory(icon_bytes)
        .map_err(|e| format!("加载图标失败: {}", e))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    let icon = Image::new_owned(img.into_raw(), width, height);

    // 构建托盘图标 - 点击激活窗口，和 Antigravity-Manager 一致
    let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Codex Switcher - 点击显示窗口")
        .on_tray_icon_event(|tray, event| {
            // 左键或右键点击都激活窗口
            if let TrayIconEvent::Click { button, .. } = event {
                if button == MouseButton::Left || button == MouseButton::Right {
                    let app = tray.app_handle();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        #[cfg(target_os = "macos")]
                        app.set_activation_policy(tauri::ActivationPolicy::Regular)
                            .unwrap_or(());
                    }
                }
            }
        })
        .build(app)?;

    println!("✅ 系统托盘已初始化 (点击激活窗口)");
    Ok(())
}
