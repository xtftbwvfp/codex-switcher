import { useState, useEffect } from 'react';

/**
 * Hook to calculate remaining time until a reset timestamp
 * @param resetAt Unix timestamp in seconds
 */
export function useCountdown(resetAt?: number) {
    const [timeLeft, setTimeLeft] = useState<string>('');

    useEffect(() => {
        if (!resetAt || resetAt <= 0) {
            setTimeLeft('');
            return;
        }

        const update = () => {
            const now = Math.floor(Date.now() / 1000);
            const diff = resetAt - now;

            if (diff <= 0) {
                setTimeLeft('即将重置');
                return;
            }

            const hours = Math.floor(diff / 3600);
            const minutes = Math.floor((diff % 3600) / 60);
            const seconds = diff % 60;

            if (hours > 24) {
                const days = Math.floor(hours / 24);
                setTimeLeft(`${days} 天后重置`);
            } else if (hours > 0) {
                setTimeLeft(`${hours}h ${minutes}m ${seconds}s 后重置`);
            } else if (minutes > 0) {
                setTimeLeft(`${minutes}m ${seconds}s 后重置`);
            } else {
                setTimeLeft(`${seconds}s 后重置`);
            }
        };

        update();
        const timer = setInterval(update, 1000);
        return () => clearInterval(timer);
    }, [resetAt]);

    return timeLeft;
}

/**
 * Minimal version of the countdown for table/list views (e.g. "4h 59m")
 */
export function useShortCountdown(resetAt?: number) {
    const [timeLeft, setTimeLeft] = useState<string>('');

    useEffect(() => {
        if (!resetAt || resetAt <= 0) {
            setTimeLeft('');
            return;
        }

        const update = () => {
            const now = Math.floor(Date.now() / 1000);
            const diff = resetAt - now;

            if (diff <= 0) {
                setTimeLeft('--');
                return;
            }

            const hours = Math.floor(diff / 3600);
            const minutes = Math.floor((diff % 3600) / 60);

            if (hours > 0) {
                setTimeLeft(`${hours}h ${minutes}m`);
            } else if (minutes > 0) {
                setTimeLeft(`${minutes}m`);
            } else {
                setTimeLeft(`${diff}s`);
            }
        };

        update();
        const timer = setInterval(update, 1000);
        return () => clearInterval(timer);
    }, [resetAt]);

    return timeLeft;
}
