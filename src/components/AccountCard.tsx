import { useState } from 'react';
import { Account } from '../hooks/useAccounts';
import './AccountCard.css';

interface AccountCardProps {
    account: Account;
    isCurrent: boolean;
    onSwitch: () => Promise<void>;
    onDelete: () => Promise<void>;
    onEdit: () => void;
}

export function AccountCard({ account, isCurrent, onSwitch, onDelete, onEdit }: AccountCardProps) {
    const [switching, setSwitching] = useState(false);
    const [deleting, setDeleting] = useState(false);
    const [confirmDelete, setConfirmDelete] = useState(false);

    const handleSwitch = async () => {
        if (isCurrent || switching) return;
        setSwitching(true);
        try {
            await onSwitch();
        } finally {
            setSwitching(false);
        }
    };

    const handleDelete = async () => {
        if (deleting) return;
        if (!confirmDelete) {
            setConfirmDelete(true);
            return;
        }
        setDeleting(true);
        try {
            await onDelete();
        } finally {
            setDeleting(false);
            setConfirmDelete(false);
        }
    };

    const formatDate = (dateStr: string | null) => {
        if (!dateStr) return '从未使用';
        const date = new Date(dateStr);
        return date.toLocaleString('zh-CN', {
            month: 'short',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
        });
    };

    return (
        <div className={`account-card ${isCurrent ? 'active' : ''}`}>
            <div className="account-status">
                <span className={`status-dot ${isCurrent ? 'active' : ''}`} />
            </div>

            <div className="account-info">
                <div className="account-header">
                    <h3 className="account-name">{account.name}</h3>
                    {isCurrent && <span className="current-badge">当前</span>}
                </div>
                <p className="account-meta">
                    使用: {formatDate(account.last_used)}
                </p>
                {account.cached_quota?.updated_at && (
                    <p className="account-meta">
                        刷新: {formatDate(account.cached_quota.updated_at)}
                    </p>
                )}
                {account.notes && (
                    <p className="account-notes">{account.notes}</p>
                )}
            </div>

            <div className="account-actions">
                {!isCurrent && (
                    <button
                        className="btn btn-primary"
                        onClick={handleSwitch}
                        disabled={switching}
                    >
                        {switching ? '切换中...' : '切换'}
                    </button>
                )}
                <button
                    className="btn btn-ghost"
                    onClick={onEdit}
                >
                    编辑
                </button>
                <button
                    className={`btn btn-danger ${confirmDelete ? 'confirm' : ''}`}
                    onClick={handleDelete}
                    disabled={deleting}
                    onMouseLeave={() => setConfirmDelete(false)}
                >
                    {deleting ? '删除中...' : confirmDelete ? '确认删除?' : '删除'}
                </button>
            </div>
        </div>
    );
}
