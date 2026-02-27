import { useState, useEffect } from 'react';
import { RefreshCw } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { useAccounts } from './hooks/useAccounts';
import { useUsage } from './hooks/useUsage';
import { AddAccountModal } from './components/AddAccountModal';
import { Dashboard } from './components/Dashboard';
import { AccountList } from './components/AccountList';
import { Settings } from './components/Settings';
import { ConfirmModal } from './components/ConfirmModal';
import './App.css';

type PageType = 'dashboard' | 'accounts' | 'settings';

function App() {
  const {
    accounts,
    currentId,
    settings,
    loading,
    error,
    refresh,
    importCurrent,
    switchTo,
    deleteAccount,
    setInactiveRefreshEnabled,
    exportAccounts,
    importAccounts,
    reloadIdeWindows,
    updateSettings,
    checkSyncConflict,
  } = useAccounts();

  const {
    usage,
    loading: usageLoading,
    error: usageError,
    refresh: refreshUsage,
  } = useUsage();

  const [currentPage, setCurrentPage] = useState<PageType>('dashboard');
  const [showAddModal, setShowAddModal] = useState(false);
  const [schedulerError, setSchedulerError] = useState<string | null>(null);

  // 冲突确认弹窗状态
  const [showConflictModal, setShowConflictModal] = useState(false);
  const [conflictAccountName, setConflictAccountName] = useState('');
  const [pendingSwitchId, setPendingSwitchId] = useState<string | null>(null);

  const currentAccount = accounts.find(a => a.id === currentId) || null;

  const classifyRefreshFailure = (reason: string): 'permanent' | 'transient' => {
    const lower = reason.toLowerCase();
    if (
      lower.includes('refresh_token_reused') ||
      lower.includes('refresh_token_invalidated') ||
      lower.includes('refresh_token_expired')
    ) {
      return 'permanent';
    }
    return 'transient';
  };

  // 监听后台调度器的账号更新事件
  useEffect(() => {
    const unlisten = listen('accounts-updated', () => {
      console.log('[Frontend] 收到后台刷新通知，重新加载账号列表');
      refresh();
    });

    return () => {
      unlisten.then(f => f());
    };
  }, [refresh]);

  // 监听后台刷新失败事件
  useEffect(() => {
    const unlisten = listen<{ account_name: string; reason: string }>('token-refresh-failed', (event) => {
      const { account_name, reason } = event.payload;
      const timestamp = new Date().toLocaleTimeString();
      const kind = classifyRefreshFailure(reason);
      if (kind === 'permanent') {
        setSchedulerError(`后台保活已停用（${account_name}，需重新登录）@ ${timestamp}`);
      } else {
        setSchedulerError(`后台保活临时失败（${account_name}）：${reason} @ ${timestamp}`);
      }
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  // 执行真正的切换逻辑
  const performSwitch = async (id: string) => {
    await switchTo(id);
    if (settings.auto_reload_ide) {
      setTimeout(async () => {
        await reloadIdeWindows(false);
      }, 300);
    }
    setTimeout(() => {
      refreshUsage();
    }, 500);
  };

  // 切换账号（带冲突检测）
  const handleSwitch = async (id: string) => {
    try {
      // 1. 检查是否有未同步的官方 Token 更新
      const conflictName = await checkSyncConflict();

      if (conflictName) {
        // 2. 如果有冲突，暂存目标 ID，弹出确认框
        setConflictAccountName(conflictName);
        setPendingSwitchId(id);
        setShowConflictModal(true);
        return;
      }

      // 3. 无冲突直接切换
      await performSwitch(id);
    } catch (err) {
      console.error('切换检查失败:', err);
      // 如果检查失败，保守起见还是直接尝试切换，或者报错（这里选择继续切换）
      await performSwitch(id);
    }
  };

  // 确认覆盖
  const handleConfirmSwitch = async () => {
    setShowConflictModal(false);
    if (pendingSwitchId) {
      await performSwitch(pendingSwitchId);
      setPendingSwitchId(null);
    }
  };

  // 取消切换
  const handleCancelSwitch = () => {
    setShowConflictModal(false);
    setPendingSwitchId(null);
  };

  const handleExport = async () => {
    try {
      const json = await exportAccounts();
      const blob = new Blob([json], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `codex-accounts-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      alert('导出失败: ' + String(err));
    }
  };

  const handleImport = async () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json';
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;

      try {
        const text = await file.text();
        await importAccounts(text);
        alert('导入成功！');
      } catch (err) {
        alert('导入失败: ' + String(err));
      }
    };
    input.click();
  };

  if (loading) {
    return (
      <div className="app">
        <div className="loading">
          <div className="spinner" />
          <p>加载中...</p>
        </div>
      </div>
    );
  }

  return (
    <div className="app">
      {/* 顶部标题栏 */}
      <header className="app-header">
        <div className="header-left">
          <h1>Codex Switcher</h1>
        </div>

        {/* 导航菜单 */}
        <nav className="header-nav">
          <button
            className={`nav-item ${currentPage === 'dashboard' ? 'active' : ''}`}
            onClick={() => setCurrentPage('dashboard')}
          >
            仪表盘
          </button>
          <button
            className={`nav-item ${currentPage === 'accounts' ? 'active' : ''}`}
            onClick={() => setCurrentPage('accounts')}
          >
            账号管理
          </button>
          <button
            className={`nav-item ${currentPage === 'settings' ? 'active' : ''}`}
            onClick={() => setCurrentPage('settings')}
          >
            设置
          </button>
        </nav>

        <div className="header-actions">
          <button className="btn btn-ghost" onClick={handleImport}>
            <span className="btn-icon">↑</span> 导入
          </button>
          <button className="btn btn-ghost" onClick={handleExport}>
            <span className="btn-icon">↓</span> 导出
          </button>
          <button className="btn btn-primary" onClick={() => setShowAddModal(true)}>
            + 添加账号
          </button>
          <button
            className="btn btn-accent"
            onClick={refreshUsage}
            disabled={usageLoading}
            title="刷新配额"
          >
            <RefreshCw className={`icon ${usageLoading ? 'spinning' : ''}`} style={{ marginRight: '8px' }} />
            {usageLoading ? '刷新中...' : '刷新配额'}
          </button>
        </div>
      </header>

      {(error || schedulerError) && (
        <div className="error-banner">
          {error && <div>{error}</div>}
          {schedulerError && <div>{schedulerError}</div>}
        </div>
      )}

      <main className="app-main">
        {currentPage === 'dashboard' ? (
          <Dashboard
            accounts={accounts}
            currentAccount={currentAccount}
            usage={usage}
            usageLoading={usageLoading}
            usageError={usageError}
            isCurrentInvalid={currentAccount?.cached_quota?.is_valid_for_cli === false}
            onSwitch={handleSwitch}
            onRefreshUsage={refreshUsage}
            onNavigateToAccounts={() => setCurrentPage('accounts')}
            onExport={handleExport}
          />
        ) : currentPage === 'accounts' ? (
          <AccountList
            accounts={accounts}
            currentId={currentId}
            settings={settings}
            onSwitch={handleSwitch}
            onDelete={deleteAccount}
            onSetInactiveRefreshEnabled={setInactiveRefreshEnabled}
            onUpdateSettings={updateSettings}
            onRefreshComplete={refresh}
          />
        ) : (
          <Settings />
        )}
      </main>

      <AddAccountModal
        isOpen={showAddModal}
        onClose={() => setShowAddModal(false)}
        onAdd={importCurrent}
        onSuccess={refresh}
      />

      <ConfirmModal
        isOpen={showConflictModal}
        title="⚠️ 登录状态冲突警告"
        message={
          <>
            <p>检测到官方 Codex 插件中存在未同步的 Token 更新。</p>
            <p>当前的账号状态与官方文件不一致：</p>
            <span className="confirm-account-name">{conflictAccountName || '当前账号'}</span>
            <p style={{ marginTop: '12px' }}>
              直接切换将<b>覆盖</b>官方插件中的当前登录状态，且无法找回这些未同步的更新。
            </p>
          </>
        }
        confirmText="确认覆盖并切换"
        cancelText="取消"
        onConfirm={handleConfirmSwitch}
        onCancel={handleCancelSwitch}
      />
    </div>
  );
}

export default App;
