import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './Settings.css';

interface AppSettings {
    auto_reload_ide: boolean;
    primary_ide: string;
    use_pkill_restart: boolean;
    background_refresh: boolean;
    refresh_interval_minutes: number;
    inactive_refresh_days: number;
}

const IDE_OPTIONS = [
    { value: 'Windsurf', label: 'Windsurf' },
    { value: 'Antigravity', label: 'Antigravity' },
    { value: 'Cursor', label: 'Cursor' },
    { value: 'VSCode', label: 'VS Code' },
    { value: 'Codex', label: 'Codex App' },
];

export function Settings() {
    const [settings, setSettings] = useState<AppSettings>({
        auto_reload_ide: false,
        primary_ide: 'Windsurf',
        use_pkill_restart: false,
        background_refresh: false,
        refresh_interval_minutes: 30,
        inactive_refresh_days: 7,
    });
    const [saving, setSaving] = useState(false);
    const [repairing, setRepairing] = useState(false);
    const [message, setMessage] = useState<string | null>(null);

    useEffect(() => {
        loadSettings();
    }, []);

    const loadSettings = async () => {
        try {
            const data = await invoke<AppSettings>('get_settings');
            setSettings(data);
        } catch (e) {
            console.error('加载设置失败:', e);
        }
    };

    const saveSettings = async () => {
        setSaving(true);
        setMessage(null);
        try {
            await invoke('update_settings', { settings });
            setMessage('✅ 设置已保存');
            setTimeout(() => setMessage(null), 3000);
        } catch (e) {
            setMessage(`❌ 保存失败: ${e}`);
        } finally {
            setSaving(false);
        }
    };

    const updateField = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => {
        setSettings(prev => ({ ...prev, [key]: value }));
    };

    const handleRepair = async () => {
        if (!confirm('这将尝试移除 Codex App 的安全隔离属性。\n\n系统可能会弹窗要求输入密码以获得权限。是否继续？')) {
            return;
        }

        setRepairing(true);
        setMessage(null);
        try {
            const ticket = await invoke<string>('request_quarantine_fix_ticket');
            await invoke('fix_codex_quarantine', { ticket });
            alert('✅ 修复成功！\n\n现在请尝试重新打开 Codex App。');
        } catch (e) {
            alert(`❌ 修复失败: ${e}`);
        } finally {
            setRepairing(false);
        }
    };


    return (
        <div className="settings-page">
            <div className="settings-header">
                <h2>设置</h2>
                <button
                    className="save-button"
                    onClick={saveSettings}
                    disabled={saving}
                >
                    {saving ? '保存中...' : '保存设置'}
                </button>
            </div>

            {message && <div className="settings-message">{message}</div>}

            <div className="settings-section">
                <h3>后台服务</h3>

                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">后台保活与同步</span>
                        <span className="setting-desc">当前账号只做权威回流；非活跃账号按独占策略保活刷新（保存后生效）</span>
                    </div>
                    <label className="toggle">
                        <input
                            type="checkbox"
                            checked={settings.background_refresh}
                            onChange={e => updateField('background_refresh', e.target.checked)}
                        />
                        <span className="toggle-slider"></span>
                        <span className="toggle-text">{settings.background_refresh ? '已开启' : '已关闭'}</span>
                    </label>
                </div>

                {settings.background_refresh && (
                    <>
                        <div className="setting-item sub-item">
                            <div className="setting-info">
                                <span className="setting-label">调度间隔（分钟）</span>
                            </div>
                            <input
                                type="number"
                                className="number-input"
                                min={5}
                                max={120}
                                value={settings.refresh_interval_minutes}
                                onChange={e => updateField('refresh_interval_minutes', parseInt(e.target.value) || 30)}
                            />
                        </div>
                        <div className="setting-item sub-item">
                            <div className="setting-info">
                                <span className="setting-label">非活跃保活阈值（天）</span>
                                <span className="setting-desc">当账号 last_refresh 超过该阈值时，调度器才会尝试保活刷新</span>
                            </div>
                            <input
                                type="number"
                                className="number-input"
                                min={1}
                                max={30}
                                value={settings.inactive_refresh_days}
                                onChange={e => updateField('inactive_refresh_days', parseInt(e.target.value) || 7)}
                            />
                        </div>
                    </>
                )}
            </div>

            <div className="settings-section">
                <h3>IDE 重载</h3>

                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">自动重载 IDE</span>
                        <span className="setting-desc">切换账号后自动重载 IDE 以应用新的 Token</span>
                    </div>
                    <label className="toggle">
                        <input
                            type="checkbox"
                            checked={settings.auto_reload_ide}
                            onChange={e => updateField('auto_reload_ide', e.target.checked)}
                        />
                        <span className="toggle-slider"></span>
                    </label>
                </div>

                {settings.auto_reload_ide && (
                    <>
                        <div className="setting-item sub-item">
                            <div className="setting-info">
                                <span className="setting-label">主力 IDE</span>
                                <span className="setting-desc">仅重载选中的 IDE</span>
                            </div>
                            <select
                                className="select-input"
                                value={settings.primary_ide}
                                onChange={e => updateField('primary_ide', e.target.value)}
                            >
                                {IDE_OPTIONS.map(opt => (
                                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                                ))}
                            </select>
                        </div>

                        <div className="setting-item sub-item">
                            <div className="setting-info">
                                <span className="setting-label">使用杀进程重启</span>
                                <span className="setting-desc">使用 pkill 方式重启（Windsurf 推荐，无需权限）</span>
                            </div>
                            <label className="toggle">
                                <input
                                    type="checkbox"
                                    checked={settings.use_pkill_restart}
                                    onChange={e => updateField('use_pkill_restart', e.target.checked)}
                                />
                                <span className="toggle-slider"></span>
                            </label>
                        </div>
                    </>
                )}
            </div>

            <div className="settings-section">
                <h3>故障修复</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">修复 Codex App 闪退</span>
                        <span className="setting-desc">移除 macOS 安全隔离属性 (需要管理员权限)</span>
                    </div>
                    <button
                        className="action-button warning"
                        onClick={handleRepair}
                        disabled={repairing}
                    >
                        {repairing ? '修复中...' : '立即修复'}
                    </button>
                </div>
            </div>
        </div>
    );
}
