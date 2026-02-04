import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface CachedQuota {
    five_hour_left: number;
    five_hour_reset: string;
    five_hour_reset_at?: number;
    weekly_left: number;
    weekly_reset: string;
    weekly_reset_at?: number;
    plan_type: string;
    is_valid_for_cli?: boolean;
    updated_at: string;
}

export interface AppSettings {
    auto_reload_ide: boolean;
    primary_ide: string;
    use_pkill_restart: boolean;
    background_refresh: boolean;
    refresh_interval_minutes: number;
    theme: string;
}

export interface Account {
    id: string;
    name: string;
    auth_json: unknown;
    created_at: string;
    last_used: string | null;
    notes: string | null;
    cached_quota: CachedQuota | null;
}

export function useAccounts() {
    const [accounts, setAccounts] = useState<Account[]>([]);
    const [currentId, setCurrentId] = useState<string | null>(null);
    const [settings, setSettings] = useState<AppSettings>({
        auto_reload_ide: false,
        primary_ide: 'Windsurf',
        use_pkill_restart: false,
        background_refresh: true,
        refresh_interval_minutes: 30,
        theme: 'light',
    });
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    // 加载账号和设置
    const loadData = useCallback(async () => {
        try {
            setLoading(true);
            setError(null);

            const [accountList, current, appSettings] = await Promise.all([
                invoke<Account[]>('get_accounts'),
                invoke<string | null>('get_current_account_id'),
                invoke<AppSettings>('get_settings'),
            ]);

            setAccounts(accountList);
            setCurrentId(current);
            setSettings(appSettings);
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
        }
    }, []);

    // 初始加载
    useEffect(() => {
        loadData();
    }, [loadData]);

    // 更新设置
    const updateSettings = useCallback(async (newSettings: AppSettings) => {
        try {
            setError(null);
            await invoke('update_settings', { settings: newSettings });
            setSettings(newSettings);
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, []);

    // ... 其他方法保持不变，但使用 loadData 替换 loadAccounts ...

    // 导入当前账号
    const importCurrent = useCallback(async (name: string, notes?: string) => {
        try {
            setError(null);
            await invoke('import_current_account', { name, notes });
            await loadData();
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 切换账号
    const switchTo = useCallback(async (id: string) => {
        try {
            setError(null);
            await invoke('switch_account', { id });
            setCurrentId(id);
            await loadData();
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 删除账号
    const deleteAccount = useCallback(async (id: string) => {
        try {
            setError(null);
            await invoke('delete_account', { id });
            await loadData();
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 更新账号
    const updateAccount = useCallback(async (id: string, name?: string, notes?: string) => {
        try {
            setError(null);
            await invoke('update_account', { id, name, notes });
            await loadData();
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 导出
    const exportAccounts = useCallback(async () => {
        try {
            return await invoke<string>('export_accounts');
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, []);

    // 导入
    const importAccounts = useCallback(async (json: string) => {
        try {
            setError(null);
            await invoke('import_accounts', { json });
            await loadData();
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 检查 Codex 登录状态
    const checkCodexLogin = useCallback(async () => {
        try {
            return await invoke<boolean>('check_codex_login');
        } catch {
            return false;
        }
    }, []);

    // 开始 OAuth 登录
    const startOAuthLogin = useCallback(async () => {
        try {
            setError(null);
            return await invoke<string>('start_oauth_login');
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, []);

    // 完成 OAuth 登录
    const finalizeOAuthLogin = useCallback(async (code: string) => {
        try {
            setError(null);
            const account = await invoke<Account>('finalize_oauth_login', { code });
            await loadData();
            return account;
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, [loadData]);

    // 重载 IDE 窗口
    const reloadIdeWindows = useCallback(async (useWindowReload: boolean = false) => {
        try {
            setError(null);
            return await invoke<string[]>('reload_ide_windows', { useWindowReload });
        } catch (err) {
            setError(String(err));
            throw err;
        }
    }, []);

    return {
        accounts,
        currentId,
        settings,
        loading,
        error,
        refresh: loadData,
        importCurrent,
        switchTo,
        deleteAccount,
        updateAccount,
        exportAccounts,
        importAccounts,
        checkCodexLogin,
        startOAuthLogin,
        finalizeOAuthLogin,
        reloadIdeWindows,
        updateSettings,
    };
}
