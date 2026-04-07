import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './Skills.css';

interface SkillApps {
    codex: boolean;
    claude: boolean;
    gemini: boolean;
    opencode: boolean;
}

interface InstalledSkill {
    id: string;
    name: string;
    description: string;
    directory: string;
    source: string;
    repo_owner: string | null;
    repo_name: string | null;
    apps: SkillApps;
    installed_at: string;
}

interface DiscoverableSkill {
    key: string;
    name: string;
    description: string;
    directory: string;
    repo_owner: string;
    repo_name: string;
    repo_branch: string;
    installed: boolean;
}

interface SkillRepo {
    owner: string;
    name: string;
    branch: string;
    enabled: boolean;
}

type Tab = 'installed' | 'discover' | 'repos';

const APPS = ['codex', 'claude', 'gemini', 'opencode'] as const;

export function Skills() {
    const [tab, setTab] = useState<Tab>('installed');
    const [installed, setInstalled] = useState<InstalledSkill[]>([]);
    const [discovered, setDiscovered] = useState<DiscoverableSkill[]>([]);
    const [repos, setRepos] = useState<SkillRepo[]>([]);
    const [search, setSearch] = useState('');
    const [loading, setLoading] = useState(false);
    const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

    // 新仓库表单
    const [newOwner, setNewOwner] = useState('');
    const [newName, setNewName] = useState('');
    const [newBranch, setNewBranch] = useState('main');

    const showMsg = (type: 'success' | 'error', text: string) => {
        setMessage({ type, text });
        setTimeout(() => setMessage(null), 5000);
    };

    const loadInstalled = async () => {
        try {
            const list = await invoke<InstalledSkill[]>('get_installed_skills');
            setInstalled(list);
        } catch (e) { console.error(e); }
    };

    const loadRepos = async () => {
        try {
            const list = await invoke<SkillRepo[]>('get_skill_repos');
            setRepos(list);
        } catch (e) { console.error(e); }
    };

    useEffect(() => {
        loadInstalled();
        loadRepos();
        // 首次启动扫描导入
        invoke<number>('scan_and_import_skills').then(count => {
            if (count > 0) {
                showMsg('success', `自动导入 ${count} 个已有 skill`);
                loadInstalled();
            }
        });
    }, []);

    const handleDiscover = async () => {
        setLoading(true);
        try {
            const list = await invoke<DiscoverableSkill[]>('discover_skills');
            setDiscovered(list);
            showMsg('success', `发现 ${list.length} 个 skill`);
        } catch (e) {
            showMsg('error', `发现失败: ${e}`);
        } finally {
            setLoading(false);
        }
    };

    const handleInstall = async (skill: DiscoverableSkill) => {
        setLoading(true);
        try {
            await invoke('install_skill', { skillJson: JSON.stringify(skill) });
            showMsg('success', `已安装 ${skill.name}`);
            await loadInstalled();
            // 标记为已安装
            setDiscovered(prev => prev.map(s => s.key === skill.key ? { ...s, installed: true } : s));
        } catch (e) {
            showMsg('error', `安装失败: ${e}`);
        } finally {
            setLoading(false);
        }
    };

    const handleUninstall = async (id: string, name: string) => {
        if (!confirm(`确定要卸载 ${name} 吗？将从所有 CLI 目录移除。`)) return;
        try {
            await invoke('uninstall_skill', { skillId: id });
            showMsg('success', `已卸载 ${name}`);
            await loadInstalled();
        } catch (e) {
            showMsg('error', `卸载失败: ${e}`);
        }
    };

    const handleToggle = async (id: string, app: string, enabled: boolean) => {
        try {
            await invoke('toggle_skill_app', { skillId: id, app, enabled });
            await loadInstalled();
        } catch (e) {
            showMsg('error', `同步失败: ${e}`);
        }
    };

    const handleAddRepo = async () => {
        if (!newOwner || !newName) return;
        try {
            await invoke('add_skill_repo', { owner: newOwner, name: newName, branch: newBranch });
            showMsg('success', `已添加 ${newOwner}/${newName}`);
            setNewOwner('');
            setNewName('');
            setNewBranch('main');
            await loadRepos();
        } catch (e) {
            showMsg('error', `${e}`);
        }
    };

    const handleRemoveRepo = async (owner: string, name: string) => {
        try {
            await invoke('remove_skill_repo', { owner, name });
            await loadRepos();
        } catch (e) {
            showMsg('error', `${e}`);
        }
    };

    const handleSyncAll = async () => {
        try {
            await invoke('sync_all_skills');
            showMsg('success', '同步完成');
        } catch (e) {
            showMsg('error', `同步失败: ${e}`);
        }
    };

    const filtered = installed.filter(s =>
        s.name.toLowerCase().includes(search.toLowerCase()) ||
        s.description.toLowerCase().includes(search.toLowerCase())
    );

    const filteredDiscover = discovered.filter(s =>
        s.name.toLowerCase().includes(search.toLowerCase()) ||
        s.description.toLowerCase().includes(search.toLowerCase())
    );

    return (
        <div className="skills-page">
            <div className="skills-header">
                <h2>Skills</h2>
                <div className="skills-tabs">
                    <button className={`tab-btn ${tab === 'installed' ? 'active' : ''}`} onClick={() => setTab('installed')}>
                        已安装 ({installed.length})
                    </button>
                    <button className={`tab-btn ${tab === 'discover' ? 'active' : ''}`} onClick={() => { setTab('discover'); if (discovered.length === 0) handleDiscover(); }}>
                        发现
                    </button>
                    <button className={`tab-btn ${tab === 'repos' ? 'active' : ''}`} onClick={() => setTab('repos')}>
                        仓库
                    </button>
                </div>
            </div>

            {message && (
                <div className={`settings-message ${message.type}`}>{message.text}</div>
            )}

            <div className="skills-search">
                <input
                    type="text"
                    placeholder="搜索 skill..."
                    value={search}
                    onChange={e => setSearch(e.target.value)}
                    className="search-input"
                />
                {tab === 'installed' && (
                    <button className="btn btn-sm btn-ghost" onClick={handleSyncAll}>全量同步</button>
                )}
                {tab === 'discover' && (
                    <button className="btn btn-sm btn-primary" onClick={handleDiscover} disabled={loading}>
                        {loading ? '扫描中...' : '刷新'}
                    </button>
                )}
            </div>

            {/* 已安装列表 */}
            {tab === 'installed' && (
                <div className="skills-list">
                    {filtered.length === 0 ? (
                        <div className="skills-empty">暂无已安装的 skill</div>
                    ) : filtered.map(skill => (
                        <div key={skill.id} className="skill-card">
                            <div className="skill-info">
                                <div className="skill-name">{skill.name}</div>
                                <div className="skill-desc">{skill.description || '无描述'}</div>
                                <div className="skill-meta">
                                    {skill.source === 'github' && skill.repo_owner && (
                                        <span className="skill-source">{skill.repo_owner}/{skill.repo_name}</span>
                                    )}
                                    {skill.source === 'local' && <span className="skill-source">本地</span>}
                                </div>
                            </div>
                            <div className="skill-apps">
                                {APPS.map(app => (
                                    <label key={app} className="app-toggle" title={app}>
                                        <input
                                            type="checkbox"
                                            checked={(skill.apps as any)[app]}
                                            onChange={e => handleToggle(skill.id, app, e.target.checked)}
                                        />
                                        <span className="app-name">{app}</span>
                                    </label>
                                ))}
                            </div>
                            <button
                                className="btn btn-sm btn-danger"
                                onClick={() => handleUninstall(skill.id, skill.name)}
                                title="卸载"
                            >
                                删除
                            </button>
                        </div>
                    ))}
                </div>
            )}

            {/* 发现列表 */}
            {tab === 'discover' && (
                <div className="skills-list">
                    {loading && <div className="skills-empty">正在扫描 GitHub 仓库...</div>}
                    {!loading && filteredDiscover.length === 0 && (
                        <div className="skills-empty">点击"刷新"从仓库发现 skill</div>
                    )}
                    {filteredDiscover.map(skill => (
                        <div key={skill.key} className="skill-card">
                            <div className="skill-info">
                                <div className="skill-name">{skill.name}</div>
                                <div className="skill-desc">{skill.description || '无描述'}</div>
                                <div className="skill-meta">
                                    <span className="skill-source">{skill.repo_owner}/{skill.repo_name}</span>
                                </div>
                            </div>
                            {skill.installed ? (
                                <span className="skill-installed-badge">已安装</span>
                            ) : (
                                <button
                                    className="btn btn-sm btn-primary"
                                    onClick={() => handleInstall(skill)}
                                    disabled={loading}
                                >
                                    安装
                                </button>
                            )}
                        </div>
                    ))}
                </div>
            )}

            {/* 仓库管理 */}
            {tab === 'repos' && (
                <div className="repos-section">
                    <div className="repo-list">
                        {repos.map(repo => (
                            <div key={`${repo.owner}/${repo.name}`} className="repo-item">
                                <div className="repo-info">
                                    <span className="repo-name">{repo.owner}/{repo.name}</span>
                                    <span className="repo-branch">{repo.branch}</span>
                                </div>
                                <button
                                    className="btn btn-sm btn-danger"
                                    onClick={() => handleRemoveRepo(repo.owner, repo.name)}
                                >
                                    移除
                                </button>
                            </div>
                        ))}
                    </div>
                    <div className="repo-add">
                        <input placeholder="owner" value={newOwner} onChange={e => setNewOwner(e.target.value)} className="repo-input" />
                        <span>/</span>
                        <input placeholder="repo" value={newName} onChange={e => setNewName(e.target.value)} className="repo-input" />
                        <input placeholder="branch" value={newBranch} onChange={e => setNewBranch(e.target.value)} className="repo-input small" />
                        <button className="btn btn-sm btn-primary" onClick={handleAddRepo}>添加</button>
                    </div>
                </div>
            )}
        </div>
    );
}
