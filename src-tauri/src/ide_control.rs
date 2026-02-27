use std::process::Command;

/// IDE 配置：名称和对应的 Bundle ID
const IDE_CONFIGS: &[(&str, &str)] = &[
    ("Visual Studio Code", "com.microsoft.VSCode"),
    ("Cursor", "com.todesktop.230313mzl4w4u92"),
    ("Windsurf", "com.exafunction.windsurf"),
    ("Antigravity", "com.google.antigravity"),
    ("Codex", "com.openai.codex"), // 暂定，用户确认后修正
];

/// 检测运行中的 IDE
pub fn detect_running_ides() -> Vec<String> {
    let mut running = Vec::new();

    for &(name, bundle_id) in IDE_CONFIGS {
        let script = format!(
            r#"
            tell application "System Events"
                if exists (every application process whose bundle identifier is "{}") then
                    return "true"
                else
                    return "false"
                end if
            end tell
            "#,
            bundle_id
        );

        if let Ok(output) = run_applescript(&script) {
            if output.trim() == "true" {
                running.push(name.to_string());
            }
        }
    }
    running
}

/// 重载指定 IDE
pub fn reload_ide(name: &str, use_window_reload: bool) -> Result<(), String> {
    // 1. 特殊处理 Windsurf: 这种 IDE 杀掉子进程后会自动重启并加载新 Token，体验最好且无需权限
    if name == "Windsurf" {
        // 尝试杀掉 Windsurf 专属的 codex 服务进程
        // 注意：使用 -f 匹配完整命令行
        let output = Command::new("pkill")
            .arg("-f")
            .arg("codex app-server")
            .output();

        match output {
            Ok(o) if o.status.success() => return Ok(()),
            _ => {
                // 如果 pkill 失败（可能进程名变了），回退到通用逻辑
                println!("Windsurf pkill 模式未匹配到进程，回退到通用模式");
            }
        }
    }

    // 2. 通用逻辑：对于其他 IDE (Antigravity, Cursor, VS Code)
    // 优先尝试模拟按键指令
    let bundle_id = IDE_CONFIGS
        .iter()
        .find(|&&(n, _)| n == name)
        .map(|&(_, b)| b)
        .ok_or_else(|| format!("未找到 IDE {} 的配置", name))?;

    let command_text = if use_window_reload {
        "Reload Window"
    } else {
        "Restart Extension Host"
    };

    // AppleScript 脚本：使用 bundle id 激活并发送指令
    let script = format!(
        r#"
        tell application id "{}"
            activate
            delay 0.5
            tell application "System Events"
                keystroke "p" using {{command down, shift down}}
                delay 0.5
                keystroke "{}"
                delay 0.5
                keystroke return
            end tell
        end tell
        "#,
        bundle_id, command_text
    );

    match run_applescript(&script) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("1002") || e.contains("不由自主") || e.contains("不允许发送按键") =>
        {
            // 捕获权限错误，返回一个友好的提示，而不是直接报错
            Err(
                "PERMISSION_DENIED:需要“辅助功能”权限来重载窗口。请手动重载或在设置中授予权限。"
                    .to_string(),
            )
        }
        Err(e) => Err(e),
    }
}

/// 移除 Codex App 的隔离属性 (修复闪退)
pub fn remove_quarantine() -> Result<(), String> {
    let script = r#"
    do shell script "xattr -dr com.apple.quarantine /Applications/Codex.app" with administrator privileges
    "#;

    run_applescript(script).map(|_| ())
}

/// 执行 AppleScript
fn run_applescript(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("无法执行 osascript: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("AppleScript 执行失败: {}", err));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
