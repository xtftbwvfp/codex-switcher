import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useAccounts } from '../hooks/useAccounts';
import './AddAccountModal.css';

interface AddAccountModalProps {
    isOpen: boolean;
    onClose: () => void;
    onAdd: (name: string, notes?: string) => Promise<void>;
    onSuccess?: () => void;  // 添加成功后的回调，用于刷新父组件列表
}

type TabType = 'official' | 'openai';

export function AddAccountModal({ isOpen, onClose, onAdd, onSuccess }: AddAccountModalProps) {
    const { startOAuthLogin, finalizeOAuthLogin } = useAccounts();
    const [activeTab, setActiveTab] = useState<TabType>('openai');
    const [name, setName] = useState('');
    const [notes, setNotes] = useState('');
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [oauthStatus, setOauthStatus] = useState<string>('');

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
        if (loading && !oauthStatus.includes('成功')) return;
        setName('');
        setNotes('');
        setError(null);
        setOauthStatus('');
        onClose();
    };

    return (
        <div className="modal-overlay" onClick={handleClose}>
            <div className="modal-content" onClick={e => e.stopPropagation()}>
                <div className="modal-header">
                    <div className="header-top">
                        <h2>添加账号</h2>
                        <button className="close-btn" onClick={handleClose} disabled={loading && !oauthStatus.includes('成功')}>
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
                    </div>
                </div>

                <div className="modal-body">
                    {activeTab === 'official' ? (
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

                            {oauthStatus && <div className="oauth-status">{oauthStatus}</div>}
                            {error && <div className="error-message" style={{ marginTop: '16px' }}>{error}</div>}

                            <div style={{ marginTop: '16px', fontSize: '12px', color: 'var(--text-tertiary)', textAlign: 'center' }}>
                                授权将在你系统的默认浏览器中完成，安全可信。
                            </div>
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
}
