use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct RefreshLockManager {
    inner: Arc<Mutex<HashSet<String>>>,
}

impl Default for RefreshLockManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl RefreshLockManager {
    pub async fn acquire(&self, account_id: &str, wait_timeout: Duration) -> bool {
        let deadline = Instant::now() + wait_timeout;
        loop {
            {
                let mut set = self.inner.lock().await;
                if !set.contains(account_id) {
                    set.insert(account_id.to_string());
                    return true;
                }
            }

            if Instant::now() >= deadline {
                return false;
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub async fn release(&self, account_id: &str) {
        let mut set = self.inner.lock().await;
        set.remove(account_id);
    }
}

#[cfg(test)]
mod tests {
    use super::RefreshLockManager;
    use tokio::time::Duration;

    #[tokio::test]
    async fn acquire_timeout_when_same_account_is_already_locked() {
        let locks = RefreshLockManager::default();
        assert!(locks.acquire("acc-1", Duration::from_millis(10)).await);
        assert!(!locks.acquire("acc-1", Duration::from_millis(10)).await);
        locks.release("acc-1").await;
    }
}
