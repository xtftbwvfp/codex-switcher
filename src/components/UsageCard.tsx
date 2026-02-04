import { UsageDisplay } from '../hooks/useUsage';
import './UsageCard.css';

interface UsageCardProps {
    usage: UsageDisplay | null;
    loading: boolean;
    error: string | null;
    onRefresh: () => void;
}

export function UsageCard({ usage, loading, error, onRefresh }: UsageCardProps) {
    if (loading && !usage) {
        return (
            <div className="usage-inline loading">
                <div className="spinner-small" />
                <span>加载用量...</span>
            </div>
        );
    }

    if (error) {
        return (
            <div className="usage-inline error">
                <span className="error-text">{error}</span>
                <button className="btn btn-ghost btn-sm" onClick={onRefresh}>
                    重试
                </button>
            </div>
        );
    }

    if (!usage) {
        return null;
    }

    return (
        <div className="usage-meters">
            {/* FREE accounts only have one quota (shown in five_hour_* fields) */}
            {usage.plan_type?.toLowerCase() === 'free' ? (
                <>
                    {/* Single quota for FREE accounts */}
                    <div className="usage-row">
                        <span className="usage-label">限额</span>
                        <span className="usage-reset">{usage.five_hour_reset}</span>
                        <span className="usage-percent">{usage.five_hour_left}%</span>
                    </div>
                    <div className="meter-bar">
                        <div
                            className={`meter-fill ${getColorClass(usage.five_hour_left)}`}
                            style={{ width: `${usage.five_hour_left}%` }}
                        />
                    </div>
                </>
            ) : (
                <>
                    {/* 5小时配额 */}
                    <div className="usage-row">
                        <span className="usage-label">5h 配额</span>
                        <span className="usage-reset">{usage.five_hour_reset}</span>
                        <span className="usage-percent">{usage.five_hour_left}%</span>
                    </div>
                    <div className="meter-bar">
                        <div
                            className={`meter-fill ${getColorClass(usage.five_hour_left)}`}
                            style={{ width: `${usage.five_hour_left}%` }}
                        />
                    </div>

                    {/* 周配额 */}
                    <div className="usage-row">
                        <span className="usage-label">周配额</span>
                        <span className="usage-reset">{usage.weekly_reset}</span>
                        <span className="usage-percent">{usage.weekly_left}%</span>
                    </div>
                    <div className="meter-bar">
                        <div
                            className={`meter-fill ${getColorClass(usage.weekly_left)}`}
                            style={{ width: `${usage.weekly_left}%` }}
                        />
                    </div>
                </>
            )}

            {/* 额度 */}
            {usage.has_credits && usage.credits_balance !== null && (
                <div className="usage-credits">
                    <span className="credits-label">💰 额度</span>
                    <span className="credits-value">${usage.credits_balance.toFixed(2)}</span>
                </div>
            )}
        </div>
    );
}

function getColorClass(percent: number): string {
    if (percent > 50) return 'green';
    if (percent > 20) return 'orange';
    return 'red';
}
