import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { UsageDisplay } from '../hooks/useUsage';
import './StatsBar.css';

interface ProxyStatus {
    enabled: boolean;
    port: number;
    is_running: boolean;
    base_url: string;
    allow_lan: boolean;
    lan_base_url?: string | null;
}

interface StatsBarProps {
    accountCount: number;
    usage: UsageDisplay | null;
}

export function StatsBar({ accountCount, usage }: StatsBarProps) {
    const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);

    const fetchProxyStatus = async () => {
        try {
            const status = await invoke<ProxyStatus>('get_proxy_status');
            setProxyStatus(status);
        } catch {
            setProxyStatus(null);
        }
    };

    useEffect(() => {
        fetchProxyStatus();
        const unlisten = listen('settings-updated', () => {
            fetchProxyStatus();
        });
        return () => { unlisten.then(fn => fn()); };
    }, []);
    return (
        <div className="stats-bar">
            <div className="stat-card">
                <div className="stat-icon blue">👤</div>
                <div className="stat-info">
                    <div className="stat-value">{accountCount}</div>
                    <div className="stat-label">账号总数</div>
                </div>
            </div>

            <div className="stat-card">
                <div className="stat-icon green">⏱</div>
                <div className="stat-info">
                    <div className="stat-value">{usage?.five_hour_left ?? '-'}%</div>
                    <div className="stat-label">5h 配额</div>
                    {usage && (
                        <div className={`stat-hint ${usage.five_hour_left > 50 ? 'good' : 'warn'}`}>
                            {usage.five_hour_left > 50 ? '配额充足' : '配额偏低'}
                        </div>
                    )}
                </div>
            </div>

            <div className="stat-card">
                <div className="stat-icon purple">📅</div>
                <div className="stat-info">
                    <div className="stat-value">{usage?.weekly_left ?? '-'}%</div>
                    <div className="stat-label">周配额</div>
                    {usage && (
                        <div className={`stat-hint ${usage.weekly_left > 50 ? 'good' : 'warn'}`}>
                            {usage.weekly_left > 50 ? '配额充足' : '配额偏低'}
                        </div>
                    )}
                </div>
            </div>

            {usage?.has_credits && (
                <div className="stat-card">
                    <div className="stat-icon gold">💰</div>
                    <div className="stat-info">
                        <div className="stat-value">${usage.credits_balance?.toFixed(2) ?? '0.00'}</div>
                        <div className="stat-label">额度余额</div>
                    </div>
                </div>
            )}

            {proxyStatus?.enabled && (
                <div className="stat-card">
                    <div className={`stat-icon ${proxyStatus.is_running ? 'green' : 'red'}`}>
                        {proxyStatus.is_running ? '🔗' : '🔌'}
                    </div>
                    <div className="stat-info">
                        <div className="stat-value">:{proxyStatus.port}</div>
                        <div className="stat-label">代理</div>
                        <div className={`stat-hint ${proxyStatus.is_running ? 'good' : 'warn'}`}>
                            {proxyStatus.is_running
                                ? proxyStatus.allow_lan ? '局域网可用' : '仅本机'
                                : '已停止'}
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
