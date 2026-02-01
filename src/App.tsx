import { useState, useEffect } from 'react';
import { RefreshCw } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { useAccounts } from './hooks/useAccounts';
import { useUsage } from './hooks/useUsage';
import { AddAccountModal } from './components/AddAccountModal';
import { Dashboard } from './components/Dashboard';
import { AccountList } from './components/AccountList';
import { Settings } from './components/Settings';
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
    exportAccounts,
    importAccounts,
    reloadIdeWindows,
    updateSettings,
  } = useAccounts();

  const {
    usage,
    loading: usageLoading,
    error: usageError,
    refresh: refreshUsage,
  } = useUsage();

  const [currentPage, setCurrentPage] = useState<PageType>('dashboard');
  const [showAddModal, setShowAddModal] = useState(false);

  const currentAccount = accounts.find(a => a.id === currentId) || null;

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

  // 切换账号后刷新用量
  const handleSwitch = async (id: string) => {
    await switchTo(id);
    if (settings.auto_reload_ide) {
      // 延迟一下等待文件写入完成
      setTimeout(async () => {
        await reloadIdeWindows(false); // false = Restart Extension Host
      }, 300);
    }
    setTimeout(() => {
      refreshUsage();
    }, 500);
  };

  const handleExport = async () => {
    try {
      const json = await exportAccounts();
      const path = await save({
        filters: [{
          name: 'JSON',
          extensions: ['json']
        }],
        defaultPath: `codex-accounts-${new Date().toISOString().slice(0, 10)}.json`
      });
      
      if (path) {
        await writeTextFile(path, json);
        alert('导出成功！文件已保存到: ' + path);
      }
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

      {error && (
        <div className="error-banner">
          {error}
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
    </div>
  );
}

export default App;
