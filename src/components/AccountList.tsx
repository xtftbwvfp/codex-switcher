import { useState, useEffect, useMemo } from 'react';
import { Zap, RefreshCw, ArrowLeftRight, Trash2, Clock } from 'lucide-react';
import { Account, AppSettings } from '../hooks/useAccounts';
import { invoke } from '@tauri-apps/api/core';
import './AccountList.css';

interface UsageData {
    five_hour_left: number;
    five_hour_reset: string;
    five_hour_reset_at?: number;
    weekly_left: number;
    weekly_reset: string;
    weekly_reset_at?: number;
    plan_type: string;
    is_valid_for_cli: boolean;
}

function QuotaTimer({ resetAt, resetStr }: { resetAt?: number, resetStr: string }) {
    const [timeLeft, setTimeLeft] = useState(resetStr);

    useEffect(() => {
        if (!resetAt) {
            setTimeLeft(resetStr);
            return;
        }

        const update = () => {
            const now = Math.floor(Date.now() / 1000);
            const diff = resetAt - now;

            if (diff <= 0) {
                setTimeLeft('Soon');
                return;
            }

            const days = Math.floor(diff / 86400);
            const hours = Math.floor((diff % 86400) / 3600);
            const mins = Math.floor((diff % 3600) / 60);
            const secs = diff % 60;

            if (days > 0) {
                // > 24h: display d h m
                setTimeLeft(`${days}d ${hours}h ${mins}m`);
            } else if (hours > 0) {
                // > 1h: display h m s
                setTimeLeft(`${hours}h ${mins}m ${secs}s`);
            } else {
                // < 1h: display m s
                setTimeLeft(`${mins}m ${secs}s`);
            }
        };

        update();
        const timer = setInterval(update, 1000);
        return () => clearInterval(timer);
    }, [resetAt, resetStr]);

    const title = resetAt
        ? new Date(resetAt * 1000).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' })
        : resetStr;

    return <span title={`重置时间: ${title}`}>{timeLeft}</span>;
}

type FilterType = 'all' | 'plus' | 'team' | 'free';

interface AccountListProps {
    accounts: Account[];
    currentId: string | null;
    settings: AppSettings;
    onSwitch: (id: string) => void;
    onDelete: (id: string) => void;
    onUpdateSettings: (settings: AppSettings) => void;
    onRefreshComplete?: () => void;  // 刷新完成后的回调
}

export function AccountList({
    accounts,
    currentId,
    settings,
    onSwitch,
    onDelete,
    onUpdateSettings,
    onRefreshComplete,
}: AccountListProps) {
    const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
    const [refreshingIds, setRefreshingIds] = useState<Set<string>>(new Set());
    const [usageMap, setUsageMap] = useState<Record<string, UsageData>>({});
    const [isRefreshingAll, setIsRefreshingAll] = useState(false);
    const [searchQuery, setSearchQuery] = useState('');
    const [filter, setFilter] = useState<FilterType>('all');
    const [invalidIds, setInvalidIds] = useState<Set<string>>(new Set()); // 无效Token的账号

    const autoReload = settings.auto_reload_ide;
    const setAutoReload = (val: boolean) => onUpdateSettings({ ...settings, auto_reload_ide: val });

    // 从 cached_quota 加载配额数据和失效状态
    useEffect(() => {
        console.log('加载 accounts 数据:', accounts.map(a => ({
            id: a.id,
            name: a.name,
            is_valid: a.cached_quota?.is_valid_for_cli
        })));

        const initial: Record<string, UsageData> = {};
        const invalidSet = new Set<string>();

        accounts.forEach(acc => {
            if (acc.cached_quota) {
                const isValid = acc.cached_quota.is_valid_for_cli !== false; // 兼容旧数据
                initial[acc.id] = {
                    five_hour_left: acc.cached_quota.five_hour_left,
                    five_hour_reset: acc.cached_quota.five_hour_reset,
                    five_hour_reset_at: acc.cached_quota.five_hour_reset_at,
                    weekly_left: acc.cached_quota.weekly_left,
                    weekly_reset: acc.cached_quota.weekly_reset,
                    weekly_reset_at: acc.cached_quota.weekly_reset_at,
                    plan_type: acc.cached_quota.plan_type,
                    is_valid_for_cli: isValid,
                };
                if (!isValid) {
                    invalidSet.add(acc.id);
                }
            }
        });
        console.log('invalidSet:', Array.from(invalidSet));
        setUsageMap(prev => ({ ...prev, ...initial }));
        setInvalidIds(invalidSet);
    }, [accounts]);

    // 搜索过滤
    const searchedAccounts = useMemo(() => {
        if (!searchQuery) return accounts;
        const lowQuery = searchQuery.toLowerCase();
        return accounts.filter(a => a.name.toLowerCase().includes(lowQuery));
    }, [accounts, searchQuery]);

    // 计算各筛选状态下的数量
    const filterCounts = useMemo(() => {
        return {
            all: searchedAccounts.length,
            plus: searchedAccounts.filter(a => {
                const usage = usageMap[a.id];
                return usage?.plan_type?.toLowerCase().includes('plus');
            }).length,
            team: searchedAccounts.filter(a => {
                const usage = usageMap[a.id];
                return usage?.plan_type?.toLowerCase().includes('team');
            }).length,
            free: searchedAccounts.filter(a => {
                const usage = usageMap[a.id];
                const tier = usage?.plan_type?.toLowerCase() || '';
                return tier && !tier.includes('plus') && !tier.includes('team');
            }).length,
        };
    }, [searchedAccounts, usageMap]);

    // 过滤结果
    const filteredAccounts = useMemo(() => {
        let result = searchedAccounts;

        if (filter === 'plus') {
            result = result.filter(a => {
                const usage = usageMap[a.id];
                return usage?.plan_type?.toLowerCase().includes('plus');
            });
        } else if (filter === 'team') {
            result = result.filter(a => {
                const usage = usageMap[a.id];
                return usage?.plan_type?.toLowerCase().includes('team');
            });
        } else if (filter === 'free') {
            result = result.filter(a => {
                const usage = usageMap[a.id];
                const tier = usage?.plan_type?.toLowerCase() || '';
                return tier && !tier.includes('plus') && !tier.includes('team');
            });
        }

        return result;
    }, [searchedAccounts, filter, usageMap]);

    // 切换单个选中
    const handleToggleSelect = (id: string) => {
        const newSet = new Set(selectedIds);
        if (newSet.has(id)) {
            newSet.delete(id);
        } else {
            newSet.add(id);
        }
        setSelectedIds(newSet);
    };

    // 全选/取消全选
    const handleToggleAll = () => {
        const currentIds = filteredAccounts.map(a => a.id);
        const allSelected = currentIds.every(id => selectedIds.has(id));

        const newSet = new Set(selectedIds);
        if (allSelected) {
            currentIds.forEach(id => newSet.delete(id));
        } else {
            currentIds.forEach(id => newSet.add(id));
        }
        setSelectedIds(newSet);
    };

    // 刷新单个账号配额
    const handleRefreshOne = async (id: string) => {
        setRefreshingIds(prev => new Set(prev).add(id));
        // 移除之前的无效状态
        setInvalidIds(prev => {
            const next = new Set(prev);
            next.delete(id);
            return next;
        });
        try {
            // 直接获取配额（不切换账号）
            const usage = await invoke<UsageData>('get_quota_by_id', { id });
            console.log('收到配额数据:', id, JSON.stringify(usage)); // Debug
            setUsageMap(prev => ({ ...prev, [id]: usage }));

            // 检查 Token 是否对 CLI 有效
            if (!usage.is_valid_for_cli) {
                console.log('检测到无效账号:', id); // Debug
                setInvalidIds(prev => new Set(prev).add(id));
            }

            // 刷新成功后，通知父组件重新加载账号列表（更新 updated_at）
            onRefreshComplete?.();
        } catch (err) {
            const errStr = String(err);
            console.error('刷新配额失败:', errStr);

            // 检测 TOKEN_INVALID 错误
            if (errStr.includes('TOKEN_INVALID')) {
                setInvalidIds(prev => new Set(prev).add(id));
            }
        } finally {
            setRefreshingIds(prev => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
        }
    };


    // 刷新选中账号
    const handleRefreshSelected = async () => {
        if (selectedIds.size === 0) {
            // 刷新全部
            setIsRefreshingAll(true);
            for (const acc of filteredAccounts) {
                await handleRefreshOne(acc.id);
            }
            setIsRefreshingAll(false);
            return;
        }

        setIsRefreshingAll(true);
        const ids = Array.from(selectedIds);

        for (const id of ids) {
            await handleRefreshOne(id);
        }

        setIsRefreshingAll(false);
    };

    // 解析中文时间描述，返回紧凑格式和总小时数
    const parseChineseDuration = (str: string | undefined) => {
        if (!str || str === '未知' || str === 'N/A') return { text: 'N/A', hours: 999 };
        if (str === '即将重置') return { text: 'Soon', hours: 0 };

        const dayMatch = str.match(/(\d+)天/);
        const hourMatch = str.match(/(\d+)小时/);
        const minMatch = str.match(/(\d+)分钟/);

        let days = 0, hours = 0, mins = 0;
        if (dayMatch) days = parseInt(dayMatch[1]);
        if (hourMatch) hours = parseInt(hourMatch[1]);
        if (minMatch) mins = parseInt(minMatch[1]);

        let totalHours = days * 24 + hours + mins / 60;

        let compactText = '';
        if (days > 0) compactText = `${days}d ${hours}h`;
        else if (hours > 0) compactText = `${hours}h ${mins}m`;
        else compactText = `${mins}m`;

        if (!compactText) compactText = 'N/A';

        return { text: compactText, hours: totalHours };
    };

    // 格式化时间
    const formatLastUsed = (date?: string | null) => {
        if (!date) return '-';
        const d = new Date(date);
        if (isNaN(d.getTime())) return '-';
        return d.toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
    };

    // 格式化剩余时间
    const formatTimeRemaining = (dateStr: string | undefined): string => {
        return parseChineseDuration(dateStr).text;
    };

    // 获取时间颜色
    const getTimeColorClass = (dateStr: string | undefined, resetAt?: number): string => {
        if (resetAt) {
            const now = Date.now() / 1000;
            const hours = (resetAt - now) / 3600;
            if (hours < 1) return 'success';
            if (hours < 6) return 'warning';
            return 'neutral';
        }
        const { hours } = parseChineseDuration(dateStr);
        if (hours === 999) return 'neutral';
        if (hours < 1) return 'success';
        if (hours < 6) return 'warning';
        return 'neutral';
    };

    // 获取配额颜色
    const getQuotaColor = (percent: number) => {
        if (percent > 50) return 'green';
        if (percent > 20) return 'orange';
        return 'red';
    };

    const renderQuotaItem = (label: string, percentage: number | undefined, resetTime: string | undefined, resetTimestamp?: number) => {
        if (percentage === undefined) return (
            <div className="quota-mini-card empty">
                <span className="quota-label">{label}</span>
                <span className="quota-empty">-</span>
            </div>
        );

        const timeColor = getTimeColorClass(resetTime, resetTimestamp);
        const barColor = getQuotaColor(percentage);

        return (
            <div className="quota-mini-card">
                {/* 进度背景层 */}
                <div
                    className={`quota-mini-bg ${barColor}`}
                    style={{ width: `${percentage}%` }}
                />

                <div className="quota-mini-content">
                    <span className="quota-label">{label}</span>
                    <div className={`quota-time ${timeColor}`}>
                        <Clock className="icon-tiny" />
                        <QuotaTimer resetAt={resetTimestamp} resetStr={formatTimeRemaining(resetTime)} />
                    </div>
                    <span className={`quota-percent ${barColor}`}>{Math.round(percentage)}%</span>
                </div>
            </div>
        );
    };

    return (
        <div className="account-list-container">
            {/* 工具栏 */}
            <div className="account-list-toolbar">
                {/* 搜索框 */}
                <div className="search-box">
                    <span className="search-icon">🔍</span>
                    <input
                        type="text"
                        placeholder="搜索邮箱..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                    />
                </div>

                {/* 类型筛选 */}
                <div className="filter-group">
                    <button
                        className={`filter-btn ${filter === 'all' ? 'active' : ''}`}
                        onClick={() => setFilter('all')}
                    >
                        全部 <span className="filter-count">{filterCounts.all}</span>
                    </button>
                    <button
                        className={`filter-btn ${filter === 'plus' ? 'active' : ''}`}
                        onClick={() => setFilter('plus')}
                    >
                        PLUS <span className="filter-count">{filterCounts.plus}</span>
                    </button>
                    <button
                        className={`filter-btn ${filter === 'team' ? 'active' : ''}`}
                        onClick={() => setFilter('team')}
                    >
                        TEAM <span className="filter-count">{filterCounts.team}</span>
                    </button>
                    <button
                        className={`filter-btn ${filter === 'free' ? 'active' : ''}`}
                        onClick={() => setFilter('free')}
                    >
                        FREE <span className="filter-count">{filterCounts.free}</span>
                    </button>
                </div>

                <div className="toolbar-spacer"></div>

                {/* 自动重载开关 */}
                <button
                    className={`btn-icon-text ${autoReload ? 'active-reload' : ''}`}
                    onClick={() => setAutoReload(!autoReload)}
                    title="切换后自动重启 Extension Host (Cmd+Shift+P -> Restart Extension Host)"
                    style={{
                        marginRight: '12px',
                        opacity: autoReload ? 1 : 0.6,
                        border: '1px solid var(--border-color)',
                        padding: '4px 8px',
                        borderRadius: '4px',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '4px',
                        background: autoReload ? 'var(--accent-color-transparent)' : 'transparent',
                        color: autoReload ? 'var(--accent-color)' : 'var(--text-secondary)'
                    }}
                >
                    <Zap size={14} fill={autoReload ? "currentColor" : "none"} />
                    <span style={{ fontSize: '12px' }}>自动重载</span>
                </button>

                {/* 刷新按钮 */}
                <button
                    className="btn-refresh"
                    onClick={handleRefreshSelected}
                    disabled={isRefreshingAll}
                    title={selectedIds.size > 0 ? `刷新选中 (${selectedIds.size})` : '刷新全部'}
                >
                    <RefreshCw className={`icon ${isRefreshingAll ? 'spinning' : ''}`} />
                </button>
            </div>


            {/* 表头 */}
            <div className="account-table-header">
                <div className="col-checkbox">
                    <input
                        type="checkbox"
                        className="custom-checkbox"
                        checked={filteredAccounts.length > 0 && filteredAccounts.every(a => selectedIds.has(a.id))}
                        onChange={handleToggleAll}
                    />
                </div>
                <div className="col-drag"></div>
                <div className="col-email">邮箱</div>
                <div className="col-quota-merged">模型配额</div>
                <div className="col-time">时间状态</div>
                <div className="col-actions">操作</div>
            </div>

            {/* 账号列表 */}
            <div className="account-table-body">
                {filteredAccounts.map(account => {
                    const usage = usageMap[account.id];
                    const isSelected = selectedIds.has(account.id);
                    const isRefreshing = refreshingIds.has(account.id);
                    const isCurrent = account.id === currentId;

                    const isInvalid = invalidIds.has(account.id);

                    return (
                        <div
                            key={account.id}
                            className={`account-row ${isSelected ? 'selected' : ''} ${isCurrent ? 'current' : ''} ${isInvalid ? 'invalid' : ''}`}
                        >
                            <div className="col-checkbox">
                                <input
                                    type="checkbox"
                                    className="custom-checkbox"
                                    checked={isSelected}
                                    onChange={() => handleToggleSelect(account.id)}
                                />
                            </div>
                            <div className="col-drag">
                                <span className="drag-handle">⋮⋮</span>
                            </div>
                            <div className="col-email">
                                <span className="email-text">{account.name}</span>
                                {isCurrent && <span className="badge current">当前</span>}
                                {isInvalid && <span className="badge invalid" title="授权已失效，请删除后重新登录">⚠️ 失效</span>}
                                {usage?.plan_type && (
                                    <span className="badge plan">{usage.plan_type.toUpperCase()}</span>
                                )}
                            </div>

                            <div className="col-quota-merged">
                                {usage ? (
                                    <div className="quota-grid">
                                        {/* FREE accounts only have one quota (shown in five_hour_* fields) */}
                                        {usage.plan_type?.toLowerCase() === 'free' ? (
                                            renderQuotaItem('限额', usage.five_hour_left, usage.five_hour_reset, usage.five_hour_reset_at)
                                        ) : (
                                            <>
                                                {renderQuotaItem('5H 限额', usage.five_hour_left, usage.five_hour_reset, usage.five_hour_reset_at)}
                                                {renderQuotaItem('周限额', usage.weekly_left, usage.weekly_reset, usage.weekly_reset_at)}
                                            </>
                                        )}
                                    </div>
                                ) : (
                                    <span className="quota-empty">- 暂无配额信息 -</span>
                                )}
                            </div>
                            <div className="col-time">
                                <div className="time-item">
                                    <span className="time-label">使用:</span>
                                    <span className="time-val">{formatLastUsed(account.last_used)}</span>
                                </div>
                                {account.cached_quota?.updated_at && (
                                    <div className="time-item refresh">
                                        <span className="time-label">刷新:</span>
                                        <span className="time-val">{formatLastUsed(account.cached_quota.updated_at)}</span>
                                    </div>
                                )}
                            </div>
                            <div className="col-actions">
                                <button
                                    className="action-btn refresh"
                                    onClick={() => handleRefreshOne(account.id)}
                                    disabled={isRefreshing}
                                    title="刷新配额"
                                >
                                    <RefreshCw className={`icon ${isRefreshing ? 'spinning' : ''}`} />
                                </button>

                                {!isCurrent && (
                                    <button
                                        className="action-btn switch"
                                        onClick={() => onSwitch(account.id)}
                                        title="切换账号"
                                    >
                                        <ArrowLeftRight className="icon" />
                                    </button>
                                )}
                                <button
                                    className="action-btn delete"
                                    onClick={() => onDelete(account.id)}
                                    title="删除账号"
                                >
                                    <Trash2 className="icon" />
                                </button>
                            </div>

                        </div>
                    );
                })}
            </div>

            {/* 底部统计 */}
            <div className="account-list-footer">
                <span>共 {filteredAccounts.length} 个账号</span>
                {selectedIds.size > 0 && (
                    <span className="selected-info">已选 {selectedIds.size} 个</span>
                )}
            </div>
        </div>
    );
}

