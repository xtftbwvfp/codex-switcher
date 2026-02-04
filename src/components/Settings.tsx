import { useState, useEffect } from 'react';
import './Settings.css';
import { AppSettings } from '../hooks/useAccounts';

interface SettingsProps {
    settings: AppSettings;
    onUpdateSettings: (settings: AppSettings) => Promise<void>;
}

const IDE_OPTIONS = [
    { value: 'Windsurf', label: 'Windsurf' },
    { value: 'Antigravity', label: 'Antigravity' },
    { value: 'Cursor', label: 'Cursor' },
    { value: 'VSCode', label: 'VS Code' },
];

const THEME_OPTIONS = [
    { value: 'light', label: '浅色 (White)' },
    { value: 'dark', label: '深色 (Dark)' },
];

export function Settings({ settings, onUpdateSettings }: SettingsProps) {
    const [localSettings, setLocalSettings] = useState<AppSettings>(settings);
    const [saving, setSaving] = useState(false);
    const [message, setMessage] = useState<string | null>(null);

    // Sync local settings when props change
    useEffect(() => {
        setLocalSettings(settings);
    }, [settings]);

    const saveSettings = async () => {
        setSaving(true);
        setMessage(null);
        try {
            await onUpdateSettings(localSettings);
            setMessage('✅ 设置已保存');
            setTimeout(() => setMessage(null), 3000);
        } catch (e) {
            setMessage(`❌ 保存失败: ${e}`);
        } finally {
            setSaving(false);
        }
    };

    const updateField = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => {
        setLocalSettings(prev => ({ ...prev, [key]: value }));
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
                <h3>外观</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">主题模式</span>
                        <span className="setting-desc">切换应用显示主题</span>
                    </div>
                    <select
                        className="select-input"
                        value={localSettings.theme || 'light'}
                        onChange={e => updateField('theme', e.target.value)}
                    >
                        {THEME_OPTIONS.map(opt => (
                            <option key={opt.value} value={opt.value}>{opt.label}</option>
                        ))}
                    </select>
                </div>
            </div>

            <div className="settings-section">
                <h3>后台服务</h3>

                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">后台自动刷新</span>
                        <span className="setting-desc">后台自动刷新所有账号的配额信息，这是 Token 保活的基础</span>
                    </div>
                    <label className="toggle always-on">
                        <input type="checkbox" checked={true} disabled />
                        <span className="toggle-slider"></span>
                        <span className="toggle-text">始终开启</span>
                    </label>
                </div>

                <div className="setting-item sub-item">
                    <div className="setting-info">
                        <span className="setting-label">刷新间隔（分钟）</span>
                    </div>
                    <input
                        type="number"
                        className="number-input"
                        min={5}
                        max={120}
                        value={localSettings.refresh_interval_minutes}
                        onChange={e => updateField('refresh_interval_minutes', parseInt(e.target.value) || 30)}
                    />
                </div>
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
                            checked={localSettings.auto_reload_ide}
                            onChange={e => updateField('auto_reload_ide', e.target.checked)}
                        />
                        <span className="toggle-slider"></span>
                    </label>
                </div>

                {localSettings.auto_reload_ide && (
                    <>
                        <div className="setting-item sub-item">
                            <div className="setting-info">
                                <span className="setting-label">主力 IDE</span>
                                <span className="setting-desc">仅重载选中的 IDE</span>
                            </div>
                            <select
                                className="select-input"
                                value={localSettings.primary_ide}
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
                                    checked={localSettings.use_pkill_restart}
                                    onChange={e => updateField('use_pkill_restart', e.target.checked)}
                                />
                                <span className="toggle-slider"></span>
                            </label>
                        </div>
                    </>
                )}
            </div>
        </div>
    );
}
