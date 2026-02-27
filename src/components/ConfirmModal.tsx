import './ConfirmModal.css';

interface ConfirmModalProps {
    isOpen: boolean;
    title: string;
    message: React.ReactNode;
    confirmText?: string;
    cancelText?: string;
    onConfirm: () => void;
    onCancel: () => void;
}

export function ConfirmModal({
    isOpen,
    title,
    message,
    confirmText = '确认',
    cancelText = '取消',
    onConfirm,
    onCancel
}: ConfirmModalProps) {
    if (!isOpen) return null;

    return (
        <div className="modal-overlay" onClick={onCancel}>
            <div className="modal-content confirm-modal" onClick={e => e.stopPropagation()}>
                <div className="confirm-header">
                    <div className="confirm-icon">⚠️</div>
                    <h3 className="confirm-title">{title}</h3>
                </div>

                <div className="confirm-body">
                    {message}
                </div>

                <div className="confirm-footer">
                    <button className="btn-cancel" onClick={onCancel}>
                        {cancelText}
                    </button>
                    <button className="btn-confirm" onClick={onConfirm}>
                        {confirmText}
                    </button>
                </div>
            </div>
        </div>
    );
}
