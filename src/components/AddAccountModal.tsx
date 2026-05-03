import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAccounts } from '../hooks/useAccounts';
import './AddAccountModal.css';

function statusIcon(status: string): string {
    switch (status) {
        case 'pending': return '·';
        case 'running': return '⟳';
        case 'ok': return '✓';
        case 'fail': return '✕';
        default: return '·';
    }
}

interface AddAccountModalProps {
    isOpen: boolean;
    onClose: () => void;
    onAdd: (name: string, notes?: string) => Promise<void>;
    onSuccess?: () => void;  // 添加成功后的回调，用于刷新父组件列表
}

type TabType = 'official' | 'openai' | 'otp_batch';

type OtpProvider = 'usmail' | 'sorryios' | 'nissanserena';

interface OtpRow {
    email: string;
    provider: OtpProvider;
    status: 'pending' | 'running' | 'ok' | 'fail';
    stage?: string;
    accountId?: string;
    error?: string;
}

interface OtpEntry {
    email: string;
    provider: OtpProvider;
    token?: string;
}

interface OtpBatchProgress {
    index: number;
    total: number;
    email: string;
    provider: OtpProvider;
    status: 'pending' | 'running' | 'ok' | 'fail';
    stage?: string;
    accountId?: string;
    error?: string;
}

/** 邮箱域名 → 默认 provider 的启发规则（没显式 token 时用） */
function pickProviderByDomain(email: string): OtpProvider {
    const domain = email.split('@')[1]?.toLowerCase() || '';
    // usmail.my.id 服务的域名
    if (domain.endsWith('daymniza.dev')) return 'usmail';
    // 其他常见印尼临时邮箱域名 → nissanserena
    if (
        domain.endsWith('.my.id') ||
        domain.endsWith('.biz.id') ||
        domain.endsWith('.web.id') ||
        domain.endsWith('.co.id')
    )
        return 'nissanserena';
    // 默认还是 usmail（兼容老用法）
    return 'usmail';
}

/**
 * 把粘贴文本解析成 OTP 任务列表。支持以下格式（混用 OK）：
 *   1. xxx@yyy.com                                              → 按域名挑 provider
 *   2. xxx@yyy.com|TOKEN32                                      → sorryios
 *   3. xxx@yyy.com|https://www.sorryios.net/token/TOKEN32       → sorryios
 *   4. 多行块包含 【账号】xxx@yyy.com ... 【验证码查询链接】...TOKEN32  → sorryios
 *   5. 73. xxx@yyy.com:password                                 → 去序号、去密码、按域名挑 provider
 */
function parseOtpEntries(raw: string): OtpEntry[] {
    const text = raw.replace(/\r\n/g, '\n');
    const tokenRe = /[0-9a-f]{32}/i;
    const emailRe = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/;
    const out: OtpEntry[] = [];
    const seen = new Set<string>();

    // 收集所有 email + 32-hex token 在文本里的位置，按距离配对（sorryios 优先）
    const emails: { email: string; idx: number }[] = [];
    const emailGlobal = new RegExp(emailRe.source, 'g');
    for (const m of text.matchAll(emailGlobal)) {
        if (m.index !== undefined) emails.push({ email: m[0], idx: m.index });
    }
    const tokenGlobal = new RegExp(tokenRe.source, 'gi');
    const tokens: { token: string; idx: number }[] = [];
    for (const m of text.matchAll(tokenGlobal)) {
        if (m.index !== undefined) tokens.push({ token: m[0].toLowerCase(), idx: m.index });
    }
    const usedTokenIdx = new Set<number>();
    for (const e of emails) {
        const key = e.email.toLowerCase();
        if (seen.has(key)) continue;
        // 找前后 400 字符内最近的 token（必须前后无字母前缀，避免误吞 chatgpt id 这种）
        let best: { token: string; idx: number; dist: number } | null = null;
        for (const t of tokens) {
            if (usedTokenIdx.has(t.idx)) continue;
            const dist = Math.abs(t.idx - e.idx);
            if (dist > 400) continue;
            if (!best || dist < best.dist) best = { ...t, dist };
        }
        if (best) {
            usedTokenIdx.add(best.idx);
            out.push({ email: e.email, provider: 'sorryios', token: best.token });
        } else {
            out.push({ email: e.email, provider: pickProviderByDomain(e.email) });
        }
        seen.add(key);
    }
    return out;
}

export function AddAccountModal({ isOpen, onClose, onAdd, onSuccess }: AddAccountModalProps) {
    const { startOAuthLogin, finalizeOAuthLogin } = useAccounts();
    const [activeTab, setActiveTab] = useState<TabType>('openai');
    const [name, setName] = useState('');
    const [notes, setNotes] = useState('');
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [oauthStatus, setOauthStatus] = useState<string>('');
    const [showPasteInput, setShowPasteInput] = useState(false);
    const [callbackInput, setCallbackInput] = useState('');
    const [submittingCallback, setSubmittingCallback] = useState(false);
    // OTP 批量授权
    const [otpInput, setOtpInput] = useState('');
    const [otpTimeout, setOtpTimeout] = useState(180);
    const [otpRows, setOtpRows] = useState<OtpRow[]>([]);
    const [otpRunning, setOtpRunning] = useState(false);
    // 当前提交：用于失败重试时，把 OtpEntry 数组（含 token）映射回 otpRows 行
    const [otpSubmission, setOtpSubmission] = useState<{
        entries: OtpEntry[];
        rowIndices: number[];
    } | null>(null);

    // 监听后端发来的授权码
    useEffect(() => {
        if (!isOpen) return;

        const unlisten = listen<string>('oauth-callback-received', async (event) => {
            const code = event.payload;
            setOauthStatus('已获取授权码，正在交换令牌...');
            try {
                await finalizeOAuthLogin(code);
                setOauthStatus('授权成功！账号已添加。');
                setLoading(false);
                // 延迟关闭模态框，让用户看到成功提示
                setTimeout(() => {
                    onSuccess?.();  // 通知父组件刷新列表
                    onClose();
                }, 1000);
            } catch (err) {
                setError(String(err));
                setOauthStatus('');
                setLoading(false);
            }
        });

        return () => {
            unlisten.then(f => f());
        };
    }, [isOpen, finalizeOAuthLogin]);

    // 监听 OTP 批量授权进度（重试时 backend 的 index 是子集索引，要翻译回原 rows index）
    useEffect(() => {
        if (!isOpen) return;
        const unlisten = listen<OtpBatchProgress>('otp-batch-progress', (event) => {
            const p = event.payload;
            setOtpRows(prev => {
                const next = [...prev];
                const targetIdx = otpSubmission?.rowIndices[p.index] ?? p.index;
                next[targetIdx] = {
                    email: p.email,
                    provider: p.provider,
                    status: p.status,
                    stage: p.stage,
                    accountId: p.accountId,
                    error: p.error,
                };
                return next;
            });
        });
        return () => {
            unlisten.then(f => f());
        };
    }, [isOpen, otpSubmission]);

    if (!isOpen) return null;

    // 处理官方导入
    const handleSubmitOfficial = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim()) {
            setError('请输入账号名称');
            return;
        }

        setLoading(true);
        setError(null);

        try {
            await onAdd(name.trim(), notes.trim() || undefined);
            handleClose();
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
        }
    };

    // 处理 OpenAI 登录
    const handleOpenAILogin = async () => {
        setLoading(true);
        setError(null);
        setOauthStatus('正在启动官方浏览器授权...');

        try {
            // 启动 OAuth 后端任务，后端会处理打开浏览器和启动监听
            await startOAuthLogin();
            setOauthStatus('请在打开的浏览器窗口中完成 OpenAI 授权...');
        } catch (err) {
            setError(String(err));
            setOauthStatus('');
            setLoading(false);
        }
    };

    const handleClose = () => {
        // OAuth 进行中也允许关闭：后端 oauth_server 下次 start 时会 abort 旧任务，无需显式取消
        setName('');
        setNotes('');
        setError(null);
        setOauthStatus('');
        setLoading(false);
        setShowPasteInput(false);
        setCallbackInput('');
        // OTP 批量进行中不重置进度，让用户回来继续看
        if (!otpRunning) {
            setOtpInput('');
            setOtpRows([]);
        }
        onClose();
    };

    // 邮箱 OTP 批量授权
    const handleOtpBatch = async () => {
        const entries = parseOtpEntries(otpInput);
        if (entries.length === 0) {
            setError('请输入至少一个有效邮箱');
            return;
        }
        setError(null);
        setOtpRunning(true);
        setOtpRows(entries.map(e => ({ email: e.email, provider: e.provider, status: 'pending' })));
        // 首次提交：rowIndices = identity (0..N-1)
        setOtpSubmission({
            entries,
            rowIndices: entries.map((_, i) => i),
        });
        try {
            await invoke('start_otp_login_batch', {
                entries,
                timeoutSecs: otpTimeout,
            });
            onSuccess?.();
        } catch (err) {
            setError(String(err));
        } finally {
            setOtpRunning(false);
        }
    };

    // 仅重跑失败的几条
    const handleRetryFailed = async () => {
        if (!otpSubmission) return;
        // 找出所有失败行的 otpRows 下标
        const failedRowIndices: number[] = [];
        otpRows.forEach((r, i) => {
            if (r.status === 'fail') failedRowIndices.push(i);
        });
        if (failedRowIndices.length === 0) return;

        // 用 email 把 row 映射回 OtpEntry（保留原 token / provider）
        const emailToEntry = new Map<string, OtpEntry>(
            otpSubmission.entries.map(e => [e.email.toLowerCase(), e])
        );
        const retryEntries: OtpEntry[] = [];
        const retryRowIndices: number[] = [];
        for (const ri of failedRowIndices) {
            const row = otpRows[ri];
            const orig = emailToEntry.get(row.email.toLowerCase());
            if (orig) {
                retryEntries.push(orig);
                retryRowIndices.push(ri);
            }
        }
        if (retryEntries.length === 0) return;

        // 把这些行重置回 pending
        setOtpRows(prev =>
            prev.map((r, i) =>
                failedRowIndices.includes(i)
                    ? { ...r, status: 'pending', stage: undefined, error: undefined, accountId: undefined }
                    : r
            )
        );
        setOtpSubmission({ entries: retryEntries, rowIndices: retryRowIndices });
        setError(null);
        setOtpRunning(true);
        try {
            await invoke('start_otp_login_batch', {
                entries: retryEntries,
                timeoutSecs: otpTimeout,
            });
            onSuccess?.();
        } catch (err) {
            setError(String(err));
        } finally {
            setOtpRunning(false);
        }
    };

    // 浏览器跳不回本机时手动提交回调链接
    const handleSubmitCallback = async () => {
        const input = callbackInput.trim();
        if (!input) return;
        setSubmittingCallback(true);
        setError(null);
        try {
            await invoke('submit_oauth_callback', { input });
            // 后端会派发 oauth-callback-received，useEffect 里的监听会走 finalize 流程
            setOauthStatus('已提交回调链接，正在交换令牌...');
            setCallbackInput('');
            setShowPasteInput(false);
        } catch (err) {
            setError(String(err));
        } finally {
            setSubmittingCallback(false);
        }
    };

    return (
        <div className="modal-overlay" onClick={handleClose}>
            <div
                className={`modal-content${activeTab === 'otp_batch' ? ' modal-wide' : ''}`}
                onClick={e => e.stopPropagation()}
            >
                <div className="modal-header">
                    <div className="header-top">
                        <h2>添加账号</h2>
                        <button className="close-btn" onClick={handleClose}>
                            ×
                        </button>
                    </div>
                    <div className="modal-tabs">
                        <button
                            className={`tab-item ${activeTab === 'openai' ? 'active' : ''}`}
                            onClick={() => !loading && setActiveTab('openai')}
                        >
                            OpenAI 登录 (推荐)
                        </button>
                        <button
                            className={`tab-item ${activeTab === 'official' ? 'active' : ''}`}
                            onClick={() => !loading && setActiveTab('official')}
                        >
                            从官方导入
                        </button>
                        <button
                            className={`tab-item ${activeTab === 'otp_batch' ? 'active' : ''}`}
                            onClick={() => !loading && !otpRunning && setActiveTab('otp_batch')}
                        >
                            邮箱 OTP 批量
                        </button>
                    </div>
                </div>

                <div className="modal-body">
                    {activeTab === 'otp_batch' ? (
                        <div className="otp-panel">
                            <h3>邮箱 OTP 批量自动授权</h3>
                            <p className="otp-desc">
                                每行一个，自动识别 provider：
                                <code>@daymniza.dev</code> → usmail.my.id；
                                <code>@*.my.id / .biz.id / .web.id</code> → nissanserena.my.id；
                                带 <code>|TOKEN</code> 或链接 → sorryios.net。
                                <br />
                                行首数字编号、<code>:密码</code>、<code>【账号】.../【验证码查询链接】...</code> 整段都识别（密码不会用，OTP 流程不需要）。
                            </p>

                            <div className="form-group">
                                <label htmlFor="otp-emails">邮箱列表（每行一个）</label>
                                <textarea
                                    id="otp-emails"
                                    className="otp-emails"
                                    value={otpInput}
                                    onChange={e => setOtpInput(e.target.value)}
                                    placeholder={'maryturner@daymniza.dev\njuliemoore@daymniza.dev'}
                                    disabled={otpRunning}
                                    rows={8}
                                    spellCheck={false}
                                />
                            </div>

                            <div className="otp-row-inline">
                                <label htmlFor="otp-timeout">每个邮箱 OTP 超时(秒)</label>
                                <input
                                    id="otp-timeout"
                                    type="number"
                                    min={30}
                                    max={600}
                                    value={otpTimeout}
                                    onChange={e => setOtpTimeout(Math.max(30, Math.min(600, Number(e.target.value) || 180)))}
                                    disabled={otpRunning}
                                />
                            </div>

                            <div className="otp-actions">
                                <button
                                    className="btn btn-primary"
                                    onClick={handleOtpBatch}
                                    disabled={otpRunning || !otpInput.trim()}
                                    type="button"
                                >
                                    {otpRunning ? '授权中…' : '开始批量授权'}
                                </button>
                                {(() => {
                                    const failedCount = otpRows.filter(r => r.status === 'fail').length;
                                    return failedCount > 0 ? (
                                        <button
                                            className="btn btn-secondary"
                                            onClick={handleRetryFailed}
                                            disabled={otpRunning}
                                            type="button"
                                        >
                                            重试失败 ({failedCount})
                                        </button>
                                    ) : null;
                                })()}
                                <button
                                    className="btn btn-ghost"
                                    onClick={handleClose}
                                    disabled={otpRunning}
                                    type="button"
                                >
                                    关闭
                                </button>
                            </div>

                            {error && <div className="error-message" style={{ marginTop: 12 }}>{error}</div>}

                            {otpRows.length > 0 && (
                                <div className="otp-progress">
                                    {otpRows.map((row, i) => (
                                        <div key={i} className={`otp-progress-row ${row.status}`}>
                                            <span className="icon">{statusIcon(row.status)}</span>
                                            <span className="email">
                                                <span className={`provider-badge provider-${row.provider}`}>
                                                    {row.provider}
                                                </span>
                                                {row.email}
                                            </span>
                                            <span className="stage">
                                                {row.status === 'running' && (row.stage || '运行中')}
                                                {row.status === 'pending' && '等待中'}
                                                {row.status === 'ok' && '已添加'}
                                                {row.status === 'fail' && (row.error ? row.error.slice(0, 80) : '失败')}
                                            </span>
                                        </div>
                                    ))}
                                </div>
                            )}

                            {!otpRunning && otpRows.length > 0 && (
                                <div className="otp-progress-summary">
                                    <span className="ok">成功 {otpRows.filter(r => r.status === 'ok').length}</span>
                                    {' / '}
                                    <span className="fail">失败 {otpRows.filter(r => r.status === 'fail').length}</span>
                                    {' / '}
                                    共 {otpRows.length}
                                </div>
                            )}
                        </div>
                    ) : activeTab === 'official' ? (
                        <form onSubmit={handleSubmitOfficial}>
                            <p className="modal-tip">
                                将从本地官方 Codex 的登录状态 (`auth.json`) 中提取认证信息。
                            </p>

                            <div className="form-group">
                                <label htmlFor="name">账号名称 *</label>
                                <input
                                    id="name"
                                    type="text"
                                    value={name}
                                    onChange={e => setName(e.target.value)}
                                    placeholder="例如：工作账号、个人账号"
                                    disabled={loading}
                                    autoFocus
                                />
                            </div>

                            <div className="form-group">
                                <label htmlFor="notes">备注</label>
                                <textarea
                                    id="notes"
                                    value={notes}
                                    onChange={e => setNotes(e.target.value)}
                                    placeholder="可选的备注信息..."
                                    disabled={loading}
                                    rows={3}
                                />
                            </div>

                            {error && <div className="error-message">{error}</div>}

                            <div className="modal-footer" style={{ padding: '16px 0 0', border: 'none' }}>
                                <button type="button" className="btn btn-ghost" onClick={handleClose} disabled={loading}>
                                    取消
                                </button>
                                <button type="submit" className="btn btn-primary" disabled={loading}>
                                    {loading ? '导入中...' : '导入当前账号'}
                                </button>
                            </div>
                        </form>
                    ) : (
                        <div className="oauth-content">
                            <div className="oauth-icon">🛡️</div>
                            <h3 style={{ marginBottom: '8px', color: 'var(--text-primary)' }}>官方 OAuth 授权</h3>
                            <p className="oauth-desc">
                                直接通过 OpenAI 官方渠道登录。支持令牌自动续期，多账号切换更稳定，无需再手动更新 `auth.json`。
                            </p>

                            <button
                                className="btn btn-primary btn-full"
                                style={{ padding: '14px' }}
                                onClick={handleOpenAILogin}
                                disabled={loading}
                            >
                                {loading && oauthStatus ? '处理中...' : '立即登录 OpenAI'}
                            </button>

                            {!loading && (
                                <button
                                    className="btn btn-ghost btn-full"
                                    style={{ marginTop: '12px' }}
                                    onClick={handleClose}
                                >
                                    取消
                                </button>
                            )}

                            {oauthStatus && <div className="oauth-status">{oauthStatus}</div>}
                            {error && <div className="error-message" style={{ marginTop: '16px' }}>{error}</div>}

                            <div style={{ marginTop: '16px', fontSize: '12px', color: 'var(--text-tertiary)', textAlign: 'center' }}>
                                授权将在你系统的默认浏览器中完成，安全可信。
                            </div>

                            {!showPasteInput ? (
                                <button
                                    className="btn btn-ghost btn-full"
                                    style={{ marginTop: '12px', fontSize: '12px' }}
                                    onClick={() => setShowPasteInput(true)}
                                    type="button"
                                >
                                    浏览器没跳回来？手动粘贴回调链接
                                </button>
                            ) : (
                                <div style={{ marginTop: '12px', textAlign: 'left' }}>
                                    <div style={{ fontSize: '12px', color: 'var(--text-secondary)', marginBottom: '6px' }}>
                                        从浏览器地址栏复制完整 URL（包含 <code>?code=...&state=...</code>）粘贴到下方：
                                    </div>
                                    <textarea
                                        className="text-input"
                                        style={{ width: '100%', minHeight: '64px', fontFamily: 'monospace', fontSize: '12px' }}
                                        value={callbackInput}
                                        onChange={e => setCallbackInput(e.target.value)}
                                        placeholder="http://localhost:1455/auth/callback?code=...&state=..."
                                        disabled={submittingCallback}
                                    />
                                    <div style={{ display: 'flex', gap: '8px', marginTop: '8px' }}>
                                        <button
                                            className="btn btn-primary"
                                            style={{ flex: 1 }}
                                            onClick={handleSubmitCallback}
                                            disabled={submittingCallback || !callbackInput.trim()}
                                            type="button"
                                        >
                                            {submittingCallback ? '提交中...' : '开始授权'}
                                        </button>
                                        <button
                                            className="btn btn-ghost"
                                            onClick={() => { setShowPasteInput(false); setCallbackInput(''); }}
                                            disabled={submittingCallback}
                                            type="button"
                                        >
                                            取消
                                        </button>
                                    </div>
                                </div>
                            )}
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
}
