//! Skills 管理模块
//!
//! SSOT 目录: ~/.codex/skills/
//! 同步到: ~/.claude/skills/, ~/.gemini/skills/, ~/.config/opencode/skills/
//! 数据存储: ~/.codex-switcher/skills.json

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ────────────────────────────────────────────────────────────────
// 数据结构
// ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillApps {
    pub codex: bool,
    pub claude: bool,
    pub gemini: bool,
    pub opencode: bool,
}

impl Default for SkillApps {
    fn default() -> Self {
        Self {
            codex: true,
            claude: false,
            gemini: false,
            opencode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory: String,
    pub source: String, // "local" | "github"
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub apps: SkillApps,
    pub installed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepo {
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverableSkill {
    pub key: String,
    pub name: String,
    pub description: String,
    pub directory: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub repo_branch: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillData {
    #[serde(default)]
    pub skills: Vec<InstalledSkill>,
    #[serde(default = "default_repos")]
    pub repos: Vec<SkillRepo>,
}

fn default_repos() -> Vec<SkillRepo> {
    vec![
        SkillRepo {
            owner: "anthropics".into(),
            name: "skills".into(),
            branch: "main".into(),
            enabled: true,
        },
        SkillRepo {
            owner: "ComposioHQ".into(),
            name: "awesome-claude-skills".into(),
            branch: "master".into(),
            enabled: true,
        },
    ]
}

impl Default for SkillData {
    fn default() -> Self {
        Self {
            skills: Vec::new(),
            repos: default_repos(),
        }
    }
}

// ────────────────────────────────────────────────────────────────
// 路径
// ────────────────────────────────────────────────────────────────

fn ssot_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".codex").join("skills")
}

fn app_skills_dir(app: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    match app {
        "codex" => Some(home.join(".codex").join("skills")),
        "claude" => Some(home.join(".claude").join("skills")),
        "gemini" => Some(home.join(".gemini").join("skills")),
        "opencode" => Some(home.join(".config").join("opencode").join("skills")),
        _ => None,
    }
}

fn data_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".codex-switcher")
        .join("skills.json")
}

// ────────────────────────────────────────────────────────────────
// SkillStore
// ────────────────────────────────────────────────────────────────

pub struct SkillStore;

impl SkillStore {
    pub fn load() -> SkillData {
        let path = data_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            SkillData::default()
        }
    }

    pub fn save(data: &SkillData) -> Result<(), String> {
        let path = data_path();
        let json = serde_json::to_string_pretty(data)
            .map_err(|e| format!("序列化失败: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("写入失败: {}", e))
    }

    /// 扫描 SSOT 目录，补录未记录的 skills
    pub fn scan_existing(data: &mut SkillData) -> usize {
        let ssot = ssot_dir();
        if !ssot.exists() {
            return 0;
        }

        let existing_dirs: std::collections::HashSet<String> = data
            .skills
            .iter()
            .map(|s| s.directory.clone())
            .collect();

        let mut imported = 0;

        if let Ok(entries) = std::fs::read_dir(&ssot) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };

                // 跳过隐藏目录
                if dir_name.starts_with('.') {
                    continue;
                }

                // 已记录则跳过
                if existing_dirs.contains(&dir_name) {
                    continue;
                }

                // 解析 SKILL.md
                let (name, description) = parse_skill_md(&path);

                data.skills.push(InstalledSkill {
                    id: format!("local:{}", dir_name),
                    name: name.unwrap_or_else(|| dir_name.clone()),
                    description: description.unwrap_or_default(),
                    directory: dir_name,
                    source: "local".into(),
                    repo_owner: None,
                    repo_name: None,
                    apps: SkillApps::default(), // codex=true, others=false
                    installed_at: Utc::now(),
                });

                imported += 1;
            }
        }

        imported
    }

    /// 同步一个 skill 到指定 app 目录
    pub fn sync_skill_to_app(directory: &str, app: &str) -> Result<(), String> {
        let source = ssot_dir().join(directory);
        if !source.exists() {
            return Err(format!("SSOT 目录不存在: {}", source.display()));
        }

        // codex 是 SSOT 本身，不需要同步
        if app == "codex" {
            return Ok(());
        }

        let target_dir = app_skills_dir(app)
            .ok_or_else(|| format!("未知的 app: {}", app))?;

        // 确保目标 skills 目录存在
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("创建目录失败: {}", e))?;

        let target = target_dir.join(directory);

        // 移除已存在的（symlink 或目录）
        if target.exists() || target.is_symlink() {
            if target.is_symlink() || target.is_file() {
                let _ = std::fs::remove_file(&target);
            } else {
                let _ = std::fs::remove_dir_all(&target);
            }
        }

        // 尝试 symlink
        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(&source, &target).is_ok() {
                return Ok(());
            }
        }

        // fallback: copy
        copy_dir_recursive(&source, &target)
            .map_err(|e| format!("复制目录失败: {}", e))
    }

    /// 从指定 app 目录移除一个 skill
    pub fn remove_skill_from_app(directory: &str, app: &str) -> Result<(), String> {
        if app == "codex" {
            // SSOT 目录不能通过 app 移除
            return Ok(());
        }

        let target_dir = match app_skills_dir(app) {
            Some(d) => d,
            None => return Ok(()),
        };

        let target = target_dir.join(directory);
        if target.is_symlink() || target.is_file() {
            let _ = std::fs::remove_file(&target);
        } else if target.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
        Ok(())
    }

    /// 切换 skill 在某 app 上的启用状态
    pub fn toggle_app(
        data: &mut SkillData,
        skill_id: &str,
        app: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let skill = data
            .skills
            .iter_mut()
            .find(|s| s.id == skill_id)
            .ok_or_else(|| format!("skill 不存在: {}", skill_id))?;

        match app {
            "codex" => skill.apps.codex = enabled,
            "claude" => skill.apps.claude = enabled,
            "gemini" => skill.apps.gemini = enabled,
            "opencode" => skill.apps.opencode = enabled,
            _ => return Err(format!("未知 app: {}", app)),
        }

        if enabled {
            Self::sync_skill_to_app(&skill.directory, app)?;
        } else {
            Self::remove_skill_from_app(&skill.directory, app)?;
        }

        Ok(())
    }

    /// 从 GitHub 仓库发现可用 skills
    pub async fn discover_skills(repos: &[SkillRepo]) -> Vec<DiscoverableSkill> {
        let mut result = Vec::new();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap();

        for repo in repos.iter().filter(|r| r.enabled) {
            let url = format!(
                "https://github.com/{}/{}/archive/refs/heads/{}.zip",
                repo.owner, repo.name, repo.branch
            );

            println!("[Skills] 扫描仓库 {}/{}...", repo.owner, repo.name);

            let resp = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => {
                    eprintln!("[Skills] 下载仓库 {}/{} 失败", repo.owner, repo.name);
                    continue;
                }
            };

            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(_) => continue,
            };

            // 解压到临时目录
            let tmp = std::env::temp_dir().join(format!("codex-skills-{}-{}", repo.owner, repo.name));
            let _ = std::fs::remove_dir_all(&tmp);
            let _ = std::fs::create_dir_all(&tmp);

            if let Err(e) = extract_zip(&bytes, &tmp) {
                eprintln!("[Skills] 解压失败: {}", e);
                continue;
            }

            // 递归扫描 SKILL.md
            let skills = scan_for_skills(&tmp, &repo.owner, &repo.name, &repo.branch);
            result.extend(skills);

            let _ = std::fs::remove_dir_all(&tmp);
        }

        result
    }

    /// 安装一个 skill（从 GitHub）
    pub async fn install_skill(
        data: &mut SkillData,
        skill: &DiscoverableSkill,
    ) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;

        let url = format!(
            "https://github.com/{}/{}/archive/refs/heads/{}.zip",
            skill.repo_owner, skill.repo_name, skill.repo_branch
        );

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("下载失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("下载失败: HTTP {}", resp.status()));
        }

        let bytes = resp.bytes().await.map_err(|e| format!("读取失败: {}", e))?;

        let tmp = std::env::temp_dir().join(format!("codex-skill-install-{}", skill.directory));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        extract_zip(&bytes, &tmp)?;

        // 找到 skill 目录
        let skill_src = find_skill_dir(&tmp, &skill.directory)
            .ok_or_else(|| format!("在仓库中未找到 skill: {}", skill.directory))?;

        // 复制到 SSOT
        let target = ssot_dir().join(&skill.directory);
        if target.exists() {
            let _ = std::fs::remove_dir_all(&target);
        }
        copy_dir_recursive(&skill_src, &target)?;

        // 记录到数据
        let installed = InstalledSkill {
            id: skill.key.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            directory: skill.directory.clone(),
            source: "github".into(),
            repo_owner: Some(skill.repo_owner.clone()),
            repo_name: Some(skill.repo_name.clone()),
            apps: SkillApps::default(),
            installed_at: Utc::now(),
        };

        // 去重
        data.skills.retain(|s| s.directory != skill.directory);
        data.skills.push(installed);

        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }

    /// 卸载 skill
    pub fn uninstall_skill(data: &mut SkillData, skill_id: &str) -> Result<(), String> {
        let skill = data
            .skills
            .iter()
            .find(|s| s.id == skill_id)
            .ok_or_else(|| format!("skill 不存在: {}", skill_id))?
            .clone();

        // 从所有 app 目录移除
        for app in &["claude", "gemini", "opencode"] {
            let _ = Self::remove_skill_from_app(&skill.directory, app);
        }

        // 从 SSOT 删除
        let ssot_path = ssot_dir().join(&skill.directory);
        if ssot_path.exists() {
            let _ = std::fs::remove_dir_all(&ssot_path);
        }

        // 从数据中移除
        data.skills.retain(|s| s.id != skill_id);

        Ok(())
    }

    /// 全量同步所有 skills 到各 app
    pub fn sync_all(data: &SkillData) {
        for skill in &data.skills {
            for (app, enabled) in [
                ("claude", skill.apps.claude),
                ("gemini", skill.apps.gemini),
                ("opencode", skill.apps.opencode),
            ] {
                if enabled {
                    let _ = Self::sync_skill_to_app(&skill.directory, app);
                } else {
                    let _ = Self::remove_skill_from_app(&skill.directory, app);
                }
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────────────────────

/// 解析 SKILL.md 的 YAML frontmatter
fn parse_skill_md(dir: &std::path::Path) -> (Option<String>, Option<String>) {
    let md_path = dir.join("SKILL.md");
    let content = match std::fs::read_to_string(&md_path) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };

    let mut name = None;
    let mut description = None;
    let mut in_frontmatter = false;

    for line in content.lines() {
        if line.trim() == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if let Some(val) = line.strip_prefix("name:") {
                name = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("description:") {
                description = Some(val.trim().to_string());
            }
        }
    }

    (name, description)
}

/// 递归扫描目录中的 SKILL.md
fn scan_for_skills(
    root: &std::path::Path,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Vec<DiscoverableSkill> {
    let mut result = Vec::new();

    fn walk(
        dir: &std::path::Path,
        owner: &str,
        repo: &str,
        branch: &str,
        result: &mut Vec<DiscoverableSkill>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    let (name, description) = parse_skill_md(&path);
                    let dir_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    if dir_name.is_empty() || dir_name.starts_with('.') {
                        continue;
                    }

                    let key = format!("{}/{}:{}", owner, repo, dir_name);

                    result.push(DiscoverableSkill {
                        key,
                        name: name.unwrap_or_else(|| dir_name.clone()),
                        description: description.unwrap_or_default(),
                        directory: dir_name,
                        repo_owner: owner.to_string(),
                        repo_name: repo.to_string(),
                        repo_branch: branch.to_string(),
                        installed: false,
                    });
                } else {
                    // 继续递归
                    walk(&path, owner, repo, branch, result);
                }
            }
        }
    }

    walk(root, owner, repo, branch, &mut result);
    result
}

/// 解压 ZIP 文件
fn extract_zip(data: &[u8], target: &std::path::Path) -> Result<(), String> {
    use std::io::{Cursor, Read, Write};

    let reader = Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("打开 ZIP 失败: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {}", e))?;

        let name = file.name().to_string();
        let out_path = target.join(&name);

        if file.is_dir() {
            let _ = std::fs::create_dir_all(&out_path);
        } else {
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("读取失败: {}", e))?;
            out.write_all(&buf)
                .map_err(|e| format!("写入失败: {}", e))?;
        }
    }

    Ok(())
}

/// 递归复制目录
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {}", e))?;

    for entry in std::fs::read_dir(src).map_err(|e| format!("读取目录失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("复制文件失败: {}", e))?;
        }
    }

    Ok(())
}

/// 在解压后的仓库目录中找到特定 skill
fn find_skill_dir(root: &std::path::Path, directory: &str) -> Option<PathBuf> {
    fn walk(dir: &std::path::Path, target: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name()?.to_str()?;
                if name == target && path.join("SKILL.md").exists() {
                    return Some(path);
                }
                if let Some(found) = walk(&path, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    walk(root, directory)
}
