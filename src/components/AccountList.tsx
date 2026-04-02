import { useState, useEffect, useMemo } from 'react';
import { Zap, RefreshCw, ArrowLeftRight, Trash2, Clock, ShieldCheck, ShieldOff } from 'lucide-react';
import { Account, AppSettings } from '../hooks/useAccounts';
import { invoke } from '@tauri-apps/api/core';
import { useShortCountdown } from '../hooks/useCountdown';
import './AccountList.css';
import { ConfirmModal } from './ConfirmModal';

interface UsageData {
    five_hour_left: number;
    five_hour_reset: string;
    five_hour_reset_at?: number;
    five_hour_label: string;
    weekly_left: number;
    weekly_reset: string;
    weekly_reset_at?: number;
    weekly_label: string;
    plan_type: string;
    is_valid_for_cli: boolean;
}

type FilterType = 'all' | 'plus' | 'team' | 'free';

interface AccountListProps {
    accounts: Account[];
    currentId: string | null;
    settings: AppSettings;
    onSwitch: (id: string) => void | Promise<void>;
    onDelete: (id: string) => void;
    onSetInactiveRefreshEnabled: (id: string, enabled: boolean) => void;
    onUpdateSettings: (settings: AppSettings) => void;
    onRefreshComplete?: () => void;
}

export function AccountList({
    accounts,
    currentId,
    settings,
    onSwitch,
    onDelete,
    onSetInactiveRefreshEnabled,
    onUpdateSettings,
    onRefreshComplete,
}: AccountListProps) {
    const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
    const [refreshingIds, setRefreshingIds] = useState<Set<string>>(new Set());
    const [copiedId, setCopiedId] = useState<string | null>(null);
    const [switchingIds] = useState<Set<string>>(new Set());
    const [usageMap, setUsageMap] = useState<Record<string, UsageData>>({});
    const [isRefreshingAll, setIsRefreshingAll] = useState(false);
    const [searchQuery, setSearchQuery] = useState('');
    const [filter, setFilter] = useState<FilterType>('all');
    const [invalidIds, setInvalidIds] = useState<Set<string>>(new Set());
    const [bannedIds, setBannedIds] = useState<Set<string>>(new Set());
    const [accountToDelete, setAccountToDelete] = useState<{ id: string, name: string } | null>(null);

    const autoReload = settings.auto_reload_ide;
    const setAutoReload = (val: boolean) => onUpdateSettings({ ...settings, auto_reload_ide: val });

    const handleCopy = (id: string, text: string) => {
        navigator.clipboard.writeText(text).then(() => {
            setCopiedId(id);
            setTimeout(() => setCopiedId(null), 2000);
        });
    };

    // 初始化数据
    useEffect(() => {
        const initialUsage: Record<string, UsageData> = {};
        const initialInvalids = new Set<string>();
        const initialBanned = new Set<string>();

        accounts.forEach(acc => {
            if (acc.is_banned) {
                initialBanned.add(acc.id);
                initialInvalids.add(acc.id);
            } else if (acc.is_token_invalid || acc.is_logged_out) {
                initialInvalids.add(acc.id);
            }
            if (acc.cached_quota) {
                const isValid = acc.cached_quota.is_valid_for_cli !== false;
                initialUsage[acc.id] = {
                    five_hour_left: acc.cached_quota.five_hour_left,
                    five_hour_reset: acc.cached_quota.five_hour_reset,
                    five_hour_reset_at: acc.cached_quota.five_hour_reset_at,
                    five_hour_label: acc.cached_quota.five_hour_label || '5H 限额',
                    weekly_left: acc.cached_quota.weekly_left,
                    weekly_reset: acc.cached_quota.weekly_reset,
                    weekly_reset_at: acc.cached_quota.weekly_reset_at,
                    weekly_label: acc.cached_quota.weekly_label || '周限额',
                    plan_type: acc.cached_quota.plan_type,
                    is_valid_for_cli: isValid,
                };
                if (!isValid) initialInvalids.add(acc.id);
            }
        });
        setUsageMap(prev => ({ ...prev, ...initialUsage }));
        setInvalidIds(initialInvalids);
        setBannedIds(initialBanned);
    }, [accounts]);

    // 搜索与过滤逻辑
    const filteredAccounts = useMemo(() => {
        let result = searchQuery
            ? accounts.filter(a => a.name.toLowerCase().includes(searchQuery.toLowerCase()))
            : accounts;

        if (filter !== 'all') {
            result = result.filter(a => {
                const type = usageMap[a.id]?.plan_type?.toLowerCase() || '';
                if (filter === 'plus') return type.includes('plus');
                if (filter === 'team') return type.includes('team');
                if (filter === 'free') return type && !type.includes('plus') && !type.includes('team');
                return true;
            });
        }
        return result;
    }, [accounts, searchQuery, filter, usageMap]);

    const filterCounts = useMemo(() => {
        const counts = { all: accounts.length, plus: 0, team: 0, free: 0 };
        accounts.forEach(a => {
            const type = usageMap[a.id]?.plan_type?.toLowerCase() || '';
            if (type.includes('plus')) counts.plus++;
            else if (type.includes('team')) counts.team++;
            else if (type) counts.free++;
        });
        return counts;
    }, [accounts, usageMap]);

    // 辅助工具函数
    const formatDate = (val?: string | Date | null) => {
        if (!val) return '-';
        const d = typeof val === 'string' ? new Date(val) : val;
        return isNaN(d.getTime()) ? '-' : d.toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
    };

    const parseDuration = (str?: string) => {
        if (!str || str === '未知' || str === 'N/A') return { text: 'N/A', hours: 999 };
        if (str === '即将重置') return { text: '重置中', hours: 0 };
        const matches = { d: str.match(/(\d+)天/), h: str.match(/(\d+)小时/), m: str.match(/(\d+)分钟/) };
        const d = parseInt(matches.d?.[1] || '0'), h = parseInt(matches.h?.[1] || '0'), m = parseInt(matches.m?.[1] || '0');
        const totalH = d * 24 + h + m / 60;
        const compact = d > 0 ? `${d}天 ${h}时` : h > 0 ? `${h}时 ${m}分` : `${m}分`;
        return { text: compact || 'N/A', hours: totalH };
    };

    const getStatusInfo = (account: Account) => {
        const isCurrent = account.id === currentId;
        const enabled = account.keepalive?.inactive_refresh_enabled !== false;
        const err = account.keepalive?.last_error;
        const isPermanent = err?.toLowerCase().match(/reused|invalidated|expired/);

        if (isPermanent) return { text: '过期', warn: true };
        if (isCurrent) return { text: '当前账号', warn: false };
        if (!enabled) return { text: '已停用', warn: true };
        return { text: err ? '重试中' : '已启用', warn: !!err };
    };

    // 交互处理
    const handleRefreshOne = async (id: string) => {
        setRefreshingIds(prev => new Set(prev).add(id));
        try {
            const usage = await invoke<UsageData>('get_quota_by_id', { id });
            setUsageMap(prev => ({ ...prev, [id]: usage }));
            setInvalidIds(prev => {
                const next = new Set(prev);
                usage.is_valid_for_cli ? next.delete(id) : next.add(id);
                return next;
            });
            onRefreshComplete?.();
        } catch (err) {
            const errMsg = String(err);
            if (errMsg.includes('ACCOUNT_BANNED')) {
                setBannedIds(prev => new Set(prev).add(id));
                setInvalidIds(prev => new Set(prev).add(id));
            } else if (errMsg.includes('TOKEN_INVALID')) {
                setInvalidIds(prev => new Set(prev).add(id));
            }
        } finally {
            setRefreshingIds(prev => { const n = new Set(prev); n.delete(id); return n; });
        }
    };

    const QuotaItem = ({ label, percentage, reset, resetAt }: { label: string, percentage: number | undefined, reset: string | undefined, resetAt?: number }) => {
        const countdown = useShortCountdown(resetAt);
        if (percentage === undefined) return (
            <div className="quota-mini-card empty">
                <span className="quota-label">{label}</span>
                <span className="quota-empty">-</span>
            </div>
        );
        const { text, hours } = parseDuration(reset);
        const displayTime = countdown || text;
        const color = percentage > 50 ? 'green' : percentage > 20 ? 'orange' : 'red';
        const timeColor = hours < 1 ? 'success' : hours < 6 ? 'warning' : 'neutral';

        return (
            <div className="quota-mini-card">
                <div className={`quota-mini-bg ${color}`} style={{ width: `${percentage}%` }} />
                <div className="quota-mini-content">
                    <span className="quota-label">{label}</span>
                    <div className={`quota-time ${timeColor}`}>
                        <Clock className="icon-tiny" />
                        <span>{displayTime}</span>
                    </div>
                    <span className={`quota-percent ${color}`}>{Math.round(percentage)}%</span>
                </div>
            </div>
        );
    };

    return (
        <div className="account-list-container">
            <div className="account-list-toolbar">
                <div className="search-box">
                    <span className="search-icon">🔍</span>
                    <input type="text" placeholder="搜索邮箱..." value={searchQuery} onChange={e => setSearchQuery(e.target.value)} />
                </div>
                <div className="filter-group">
                    {(['all', 'plus', 'team', 'free'] as const).map(t => (
                        <button key={t} className={`filter-btn ${filter === t ? 'active' : ''}`} onClick={() => setFilter(t)}>
                            {t.toUpperCase()} <span className="filter-count">{filterCounts[t]}</span>
                        </button>
                    ))}
                </div>
                <div className="toolbar-spacer" />
                <button
                    className={`btn-icon-text ${autoReload ? 'active-reload' : ''}`}
                    onClick={() => setAutoReload(!autoReload)}
                    style={{
                        marginRight: '12px',
                        padding: '4px 8px',
                        borderRadius: '4px',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '4px',
                        background: autoReload ? 'var(--badge-bg)' : 'transparent',
                        color: autoReload ? 'var(--primary-color)' : 'var(--text-muted)',
                        border: '1px solid var(--border-color)'
                    }}
                >
                    <Zap size={14} fill={autoReload ? "currentColor" : "none"} />
                    <span style={{ fontSize: '12px' }}>自动重载</span>
                </button>
                <button className="btn-refresh" onClick={() => { setIsRefreshingAll(true); Promise.all(filteredAccounts.map(a => handleRefreshOne(a.id))).finally(() => setIsRefreshingAll(false)); }}>
                    <RefreshCw className={isRefreshingAll ? 'spinning' : ''} size={16} />
                </button>
            </div>

            <div className="account-table-scroll">
                <div className="account-table-header">
                    <div className="col-checkbox">
                        <input type="checkbox" className="custom-checkbox" checked={filteredAccounts.length > 0 && filteredAccounts.every(a => selectedIds.has(a.id))} onChange={() => { const s = new Set(selectedIds); filteredAccounts.every(a => s.has(a.id)) ? filteredAccounts.forEach(a => s.delete(a.id)) : filteredAccounts.forEach(a => s.add(a.id)); setSelectedIds(s); }} />
                    </div>
                    <div className="col-drag"></div>
                    <div className="col-email">账号信息</div>
                    <div className="col-quota-merged">配额状态</div>
                    <div className="col-time">同步/保活</div>
                    <div className="col-actions">操作</div>
                </div>

                <div className="account-table-body">
                    {filteredAccounts.map(acc => {
                        const usage = usageMap[acc.id];
                        const status = getStatusInfo(acc);
                        const err = acc.keepalive?.last_error;
                        const isPermanentError = err?.toLowerCase().match(/reused|invalidated|expired/);
                        const isInvalid = invalidIds.has(acc.id) || !!isPermanentError || acc.is_token_invalid || acc.is_logged_out;
                        const isBanned = bannedIds.has(acc.id);
                        const isLoggedOut = acc.is_logged_out;
                        const isCurrent = acc.id === currentId;
                        const isRefreshing = refreshingIds.has(acc.id);

                        return (
                            <div key={acc.id} className={`account-row ${isCurrent ? 'current' : ''} ${selectedIds.has(acc.id) ? 'selected' : ''} ${isBanned ? 'banned' : isLoggedOut ? 'logged-out' : isInvalid ? 'expired' : ''}`}>
                                <div className="col-checkbox">
                                    <input type="checkbox" className="custom-checkbox" checked={selectedIds.has(acc.id)} onChange={() => { const s = new Set(selectedIds); s.has(acc.id) ? s.delete(acc.id) : s.add(acc.id); setSelectedIds(s); }} />
                                </div>
                                <div className="col-drag"><span className="drag-handle">⋮⋮</span></div>
                                <div className="col-email" onClick={() => handleCopy(acc.id, acc.name)} title="点击复制账号">
                                    <span className="email-text">{acc.name}</span>
                                    <div className="badges" style={{ display: 'flex', gap: '4px', marginLeft: '8px' }}>
                                        {copiedId === acc.id && <span className="badge copy-success">已复制</span>}
                                        {isCurrent && <span className="badge current">当前</span>}
                                        {isBanned ? <span className="badge banned" title="该账号已被 OpenAI 封禁">封号</span> : isLoggedOut ? <span className="badge logged-out" title="您已登出或登录了其他账号，请重新登录">已登出</span> : isInvalid && <span className="badge expired" title="该账号 Token 已过期或失效">过期</span>}
                                        {usage?.plan_type && <span className="badge plan">{usage.plan_type.toUpperCase()}</span>}
                                    </div>
                                </div>
                                <div className="col-quota-merged">
                                    {usage ? (
                                        <div className="quota-grid">
                                            <QuotaItem label={usage.five_hour_label} percentage={usage.five_hour_left} reset={usage.five_hour_reset} resetAt={usage.five_hour_reset_at} />
                                            <QuotaItem label={usage.weekly_label} percentage={usage.weekly_left} reset={usage.weekly_reset} resetAt={usage.weekly_reset_at} />
                                        </div>
                                    ) : <span className="quota-empty">未获取数据</span>}
                                </div>
                                <div className="col-time">
                                    <div className="time-item">
                                        <span className="time-label">保活:</span>
                                        <span className={`time-val ${status.warn ? 'warn' : ''}`}>{status.text}</span>
                                    </div>
                                    <div className="time-item refresh">
                                        <span className="time-label">刷新:</span>
                                        <span className="time-val">{formatDate(acc.cached_quota?.updated_at)}</span>
                                    </div>
                                </div>
                                <div className="col-actions">
                                    <button className="action-btn refresh" onClick={() => handleRefreshOne(acc.id)} disabled={isRefreshing} title="刷新"><RefreshCw size={14} className={isRefreshing ? 'spinning' : ''} /></button>
                                    {!isCurrent && (
                                        <button className={`action-btn keepalive ${acc.keepalive?.inactive_refresh_enabled !== false ? 'on' : 'off'}`} onClick={() => onSetInactiveRefreshEnabled(acc.id, acc.keepalive?.inactive_refresh_enabled === false)} title="保活">
                                            {acc.keepalive?.inactive_refresh_enabled !== false ? <ShieldCheck size={14} /> : <ShieldOff size={14} />}
                                        </button>
                                    )}
                                    {!isCurrent && (
                                        <button className="action-btn switch" onClick={() => onSwitch(acc.id)} disabled={switchingIds.has(acc.id)} title="切换"><ArrowLeftRight size={14} /></button>
                                    )}
                                    <button className="action-btn delete" onClick={() => setAccountToDelete({ id: acc.id, name: acc.name })} title="删除"><Trash2 size={14} /></button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            </div>

            <div className="account-list-footer">
                <span>共 {filteredAccounts.length} 个账号</span>
                {selectedIds.size > 0 && <span className="selected-info">已选 {selectedIds.size} 个</span>}
            </div>

            <ConfirmModal
                isOpen={!!accountToDelete}
                title="确认删除账号"
                message={<p>确定要永久删除账号 <strong>{accountToDelete?.name}</strong> 吗？<br /><br />此操作不可恢复，删除后有关该账号的本地授权信息将被清除。</p>}
                confirmText="彻底删除"
                onConfirm={() => {
                    if (accountToDelete) {
                        onDelete(accountToDelete.id);
                        setAccountToDelete(null);
                    }
                }}
                onCancel={() => setAccountToDelete(null)}
            />
        </div>
    );
}
