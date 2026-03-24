import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Copy, Check, Save } from 'lucide-react';
import './Proxy.css';

interface ProxyStatus {
    enabled: boolean;
    port: number;
    is_running: boolean;
    base_url: string;
    total_requests: number;
    auto_switches: number;
}

interface AppSettings {
    auto_reload_ide: boolean;
    primary_ide: string;
    use_pkill_restart: boolean;
    background_refresh: boolean;
    refresh_interval_minutes: number;
    inactive_refresh_days: number;
    theme_palette: string;
    allow_auto_switch_to_free: boolean;
    proxy_enabled: boolean;
    proxy_port: number;
    proxy_threshold_5h: number;
    proxy_threshold_weekly: number;
    proxy_free_guard: number;
}

interface TokenStats {
    total_input_tokens: number;
    total_cached_input_tokens: number;
    total_output_tokens: number;
    total_tokens: number;
    total_cost_usd: number;
    total_requests: number;
    since: string;
    last_month_cost: number | null;
    last_month_tokens: number | null;
}

function formatTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return n.toString();
}

export function Proxy() {
    const [status, setStatus] = useState<ProxyStatus | null>(null);
    const [settings, setSettings] = useState<AppSettings | null>(null);
    const [tokenStats, setTokenStats] = useState<TokenStats | null>(null);
    const [port, setPort] = useState(18080);
    const [copied, setCopied] = useState(false);
    const [saving, setSaving] = useState(false);
    const [envWriting, setEnvWriting] = useState(false);
    const [killing, setKilling] = useState(false);
    const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
    const [switchedAccount, setSwitchedAccount] = useState<string | null>(null);
    const [fastMode, setFastMode] = useState(false);

    const fetchAll = async () => {
        try {
            const [s, st, ts, fm] = await Promise.all([
                invoke<AppSettings>('get_settings'),
                invoke<ProxyStatus>('get_proxy_status'),
                invoke<TokenStats>('get_token_stats'),
                invoke<boolean>('get_codex_fast_mode'),
            ]);
            setSettings(s);
            setFastMode(fm);
            setStatus(st);
            setTokenStats(ts);
            setPort(s.proxy_port);
        } catch (e) {
            console.error('加载代理状态失败:', e);
        }
    };

    useEffect(() => {
        fetchAll();
        const unsub1 = listen('settings-updated', fetchAll);
        const unsub2 = listen<string>('proxy-account-switched', (e) => {
            setSwitchedAccount(e.payload);
            setTimeout(() => setSwitchedAccount(null), 5000);
            fetchAll();
        });
        const unsub3 = listen<string>('proxy-all-exhausted', (e) => {
            setMessage({ type: 'error', text: e.payload });
        });
        return () => {
            unsub1.then(fn => fn());
            unsub2.then(fn => fn());
            unsub3.then(fn => fn());
        };
    }, []);

    const toggleProxy = async (enabled: boolean) => {
        if (!settings) return;
        setSaving(true);
        setMessage(null);
        try {
            await invoke('update_settings', {
                settings: { ...settings, proxy_enabled: enabled, proxy_port: port },
            });
            setMessage({ type: 'success', text: enabled ? '代理已启动' : '代理已停止' });
            setTimeout(() => setMessage(null), 3000);
        } catch (e) {
            setMessage({ type: 'error', text: `操作失败: ${e}` });
        } finally {
            setSaving(false);
        }
    };

    const savePort = async () => {
        if (!settings) return;
        setSaving(true);
        try {
            await invoke('update_settings', {
                settings: { ...settings, proxy_port: port },
            });
            setMessage({ type: 'success', text: '端口已更新（重启代理后生效）' });
            setTimeout(() => setMessage(null), 3000);
        } catch (e) {
            setMessage({ type: 'error', text: `保存失败: ${e}` });
        } finally {
            setSaving(false);
        }
    };

    const handleSetEnv = async (enable: boolean) => {
        setEnvWriting(true);
        setMessage(null);
        try {
            const result = await invoke<string>('set_proxy_env', { port, enable });
            setMessage({ type: 'success', text: result + '（新终端窗口生效）' });
            setTimeout(() => setMessage(null), 5000);
        } catch (e) {
            setMessage({ type: 'error', text: `${e}` });
        } finally {
            setEnvWriting(false);
        }
    };

    const handleKill = async () => {
        setKilling(true);
        try {
            const result = await invoke<string>('kill_codex_processes');
            setMessage({ type: 'success', text: result });
            setTimeout(() => setMessage(null), 3000);
        } catch (e) {
            setMessage({ type: 'error', text: `${e}` });
        } finally {
            setKilling(false);
        }
    };

    const isRunning = status?.is_running ?? false;
    const isEnabled = settings?.proxy_enabled ?? false;

    return (
        <div className="proxy-page">
            <div className="proxy-header">
                <h2>代理服务</h2>
                <div className={`proxy-status-badge ${isRunning ? 'running' : 'stopped'}`}>
                    <span className="status-dot" />
                    {isRunning ? '运行中' : '已停止'}
                </div>
            </div>

            {message && (
                <div className={`settings-message ${message.type}`}>
                    {message.text}
                </div>
            )}

            {switchedAccount && (
                <div className="settings-message success">
                    代理已自动切换到账号: {switchedAccount}
                </div>
            )}

            {/* Token 用量统计卡片 */}
            {tokenStats && tokenStats.total_tokens > 0 && (
                <div className="usage-cards">
                    <div className="usage-card cost">
                        <div className="usage-card-icon">$</div>
                        <div className="usage-card-content">
                            <div className="usage-card-value">${tokenStats.total_cost_usd.toFixed(2)}</div>
                            <div className="usage-card-label">Spent</div>
                            {tokenStats.last_month_cost !== null && (
                                <div className="usage-card-compare">
                                    Vs 上月 ${tokenStats.last_month_cost.toFixed(2)}
                                </div>
                            )}
                        </div>
                    </div>
                    <div className="usage-card tokens">
                        <div className="usage-card-icon">#</div>
                        <div className="usage-card-content">
                            <div className="usage-card-value">{formatTokens(tokenStats.total_tokens)}</div>
                            <div className="usage-card-label">Tokens</div>
                            <div className="usage-card-detail">
                                In {formatTokens(tokenStats.total_input_tokens)} / Out {formatTokens(tokenStats.total_output_tokens)}
                            </div>
                        </div>
                    </div>
                    <div className="usage-card requests">
                        <div className="usage-card-icon">~</div>
                        <div className="usage-card-content">
                            <div className="usage-card-value">{tokenStats.total_requests}</div>
                            <div className="usage-card-label">Requests</div>
                        </div>
                    </div>
                </div>
            )}

            {/* 代理开关 */}
            <div className="settings-section">
                <h3>代理控制</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">本地代理服务器</span>
                        <span className="setting-desc">
                            Codex CLI 通过代理连接 OpenAI，支持无中断自动切号和 429 智能重试
                        </span>
                    </div>
                    <button
                        className={`proxy-toggle-btn ${isEnabled ? 'on' : 'off'}`}
                        onClick={() => toggleProxy(!isEnabled)}
                        disabled={saving}
                    >
                        {saving ? '...' : isEnabled ? '关闭代理' : '启动代理'}
                    </button>
                </div>

                <div className="setting-item sub-item">
                    <div className="setting-info">
                        <span className="setting-label">代理端口</span>
                    </div>
                    <div className="port-input-group">
                        <input
                            type="number"
                            className="number-input"
                            min={1024}
                            max={65535}
                            value={port}
                            onChange={e => setPort(parseInt(e.target.value) || 18080)}
                        />
                        {port !== settings?.proxy_port && (
                            <button className="btn btn-sm btn-primary" onClick={savePort} disabled={saving}>
                                <Save size={12} /> 保存
                            </button>
                        )}
                    </div>
                </div>
            </div>

            {/* 环境变量配置 */}
            <div className="settings-section">
                <h3>环境变量</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">手动启动</span>
                        <span className="setting-desc">复制命令到终端运行</span>
                    </div>
                    <button
                        className="copy-command-button"
                        onClick={() => {
                            navigator.clipboard.writeText(
                                `OPENAI_BASE_URL=http://localhost:${port}/v1 codex`
                            );
                            setCopied(true);
                            setTimeout(() => setCopied(false), 2000);
                        }}
                    >
                        <code>OPENAI_BASE_URL=http://localhost:{port}/v1 codex</code>
                        {copied ? <Check size={12} /> : <Copy size={12} />}
                    </button>
                </div>

                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">全局代理（所有终端生效）</span>
                        <span className="setting-desc">
                            写入 ~/.zshrc 和 ~/.bashrc，新打开的终端中所有 codex 命令自动走代理
                        </span>
                    </div>
                    <div className="env-btn-group">
                        <button
                            className="btn btn-sm btn-primary"
                            onClick={() => handleSetEnv(true)}
                            disabled={envWriting}
                        >
                            {envWriting ? '...' : '写入环境变量'}
                        </button>
                        <button
                            className="btn btn-sm btn-ghost"
                            onClick={() => handleSetEnv(false)}
                            disabled={envWriting}
                        >
                            移除
                        </button>
                    </div>
                </div>
            </div>

            {/* Codex 配置 */}
            <div className="settings-section">
                <h3>Codex 配置</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">Fast 模式</span>
                        <span className="setting-desc">
                            更快推理速度，但消耗 2x 额度。{fastMode ? '当前：已开启' : '当前：已关闭'}
                        </span>
                    </div>
                    <button
                        className={`proxy-toggle-btn ${fastMode ? 'on' : 'off'}`}
                        onClick={async () => {
                            try {
                                const result = await invoke<string>('set_codex_fast_mode', { enable: !fastMode });
                                setFastMode(!fastMode);
                                setMessage({ type: 'success', text: result });
                                setTimeout(() => setMessage(null), 3000);
                            } catch (e) {
                                setMessage({ type: 'error', text: `${e}` });
                            }
                        }}
                    >
                        {fastMode ? '关闭 Fast' : '开启 Fast'}
                    </button>
                </div>
            </div>

            {/* 进程管理 */}
            <div className="settings-section">
                <h3>进程管理</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">终止所有 Codex 进程</span>
                        <span className="setting-desc">
                            强制终止所有运行中的 codex 进程，用于切换代理模式后重启或排错
                        </span>
                    </div>
                    <button
                        className="action-button warning"
                        onClick={handleKill}
                        disabled={killing}
                    >
                        {killing ? '终止中...' : '终止进程'}
                    </button>
                </div>
            </div>

            {/* 智能切号策略 */}
            <div className="settings-section">
                <h3>智能切号策略</h3>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">5h 配额预防性切号阈值</span>
                        <span className="setting-desc">剩余配额低于此百分比时提前切号（0 = 仅 429 触发，推荐 10）</span>
                    </div>
                    <div className="threshold-input-group">
                        <input
                            type="number"
                            className="number-input"
                            min={0}
                            max={50}
                            value={settings?.proxy_threshold_5h ?? 0}
                            onChange={e => {
                                if (!settings) return;
                                const val = parseInt(e.target.value) || 0;
                                const updated = { ...settings, proxy_threshold_5h: val };
                                setSettings(updated);
                                invoke('update_settings', { settings: updated });
                            }}
                        />
                        <span className="threshold-unit">%</span>
                    </div>
                </div>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">周配额预防性切号阈值</span>
                        <span className="setting-desc">剩余周配额低于此百分比时提前切号（0 = 仅 429 触发，推荐 5）</span>
                    </div>
                    <div className="threshold-input-group">
                        <input
                            type="number"
                            className="number-input"
                            min={0}
                            max={50}
                            value={settings?.proxy_threshold_weekly ?? 0}
                            onChange={e => {
                                if (!settings) return;
                                const val = parseInt(e.target.value) || 0;
                                const updated = { ...settings, proxy_threshold_weekly: val };
                                setSettings(updated);
                                invoke('update_settings', { settings: updated });
                            }}
                        />
                        <span className="threshold-unit">%</span>
                    </div>
                </div>
                <div className="setting-item">
                    <div className="setting-info">
                        <span className="setting-label">Free 账号保护线</span>
                        <span className="setting-desc">Free 账号剩余配额低于此百分比时强制切号（0 = 不特殊处理，推荐 35）</span>
                    </div>
                    <div className="threshold-input-group">
                        <input
                            type="number"
                            className="number-input"
                            min={0}
                            max={80}
                            value={settings?.proxy_free_guard ?? 0}
                            onChange={e => {
                                if (!settings) return;
                                const val = parseInt(e.target.value) || 0;
                                const updated = { ...settings, proxy_free_guard: val };
                                setSettings(updated);
                                invoke('update_settings', { settings: updated });
                            }}
                        />
                        <span className="threshold-unit">%</span>
                    </div>
                </div>
            </div>

            {/* 运行指标 + 工作原理 */}
            <div className="settings-section">
                <h3>运行状态</h3>
                <div className="proxy-info-card">
                    {isRunning && (
                        <>
                            <div className="info-row">
                                <span className="info-label">总请求数</span>
                                <span>{status?.total_requests ?? 0}</span>
                            </div>
                            <div className="info-row">
                                <span className="info-label">自动切号次数</span>
                                <span>{status?.auto_switches ?? 0}</span>
                            </div>
                        </>
                    )}
                    <div className="info-row">
                        <span className="info-label">监听地址</span>
                        <code>127.0.0.1:{port}</code>
                    </div>
                    <div className="info-row">
                        <span className="info-label">上游地址</span>
                        <code>https://api.openai.com</code>
                    </div>
                    <div className="info-row">
                        <span className="info-label">Token 来源</span>
                        <span>当前激活账号（由 Codex CLI 维护刷新，代理实时回读）</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label">健康检查</span>
                        <code>GET http://localhost:{port}/health</code>
                    </div>
                </div>
            </div>
        </div>
    );
}
