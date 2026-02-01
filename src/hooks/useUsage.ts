import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface UsageDisplay {
    plan_type: string;
    five_hour_used: number;
    five_hour_left: number;
    five_hour_reset: string;
    five_hour_reset_at?: number;
    weekly_used: number;
    weekly_left: number;
    weekly_reset: string;
    weekly_reset_at?: number;
    credits_balance: number | null;
    has_credits: boolean;
}

export function useUsage() {
    const [usage, setUsage] = useState<UsageDisplay | null>(null);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const fetchUsage = useCallback(async () => {
        setLoading(true);
        setError(null);

        try {
            // 先获取当前账号 ID
            const currentId = await invoke<string | null>('get_current_account_id');
            if (!currentId) {
                setError('未设置当前账号');
                return;
            }
            const data = await invoke<UsageDisplay>('get_quota_by_id', { id: currentId });
            setUsage(data);
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
        }
    }, []);

    // 初始加载
    useEffect(() => {
        fetchUsage();
    }, [fetchUsage]);

    return {
        usage,
        loading,
        error,
        refresh: fetchUsage,
    };
}
