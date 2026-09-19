//! A tiny per-key async mutex map: locking the same key serializes
//! concurrent callers so only one does the actual work, and the others see
//! its (now fresh) result via the caller's own cache re-check inside the
//! lock — the same effect as Go's `singleflight.Group.Do`, without a
//! dedicated crate. Used for JioTV's live-URL recovery fetch
//! (`refreshChannelToken` in the Go tree) and JioTV+'s
//! catalogue/token-refresh/playback caches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct KeyedLocks {
    map: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl KeyedLocks {
    pub async fn lock(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let m = {
            let mut map = self.map.lock().unwrap();
            map.entry(key.to_string()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        m.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn concurrent_callers_for_the_same_key_run_one_at_a_time() {
        let locks = Arc::new(KeyedLocks::default());
        let counter = Arc::new(AtomicU32::new(0));
        let max_concurrent = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let locks = locks.clone();
            let counter = counter.clone();
            let max_concurrent = max_concurrent.clone();
            handles.push(tokio::spawn(async move {
                let _guard = locks.lock("shared-key").await;
                let now = counter.fetch_add(1, Ordering::SeqCst) + 1;
                max_concurrent.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                counter.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(max_concurrent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_do_not_block_each_other() {
        let locks = Arc::new(KeyedLocks::default());
        let g1 = locks.lock("a").await;
        // A different key must not block on the same await point.
        let g2 = tokio::time::timeout(std::time::Duration::from_millis(50), locks.lock("b")).await;
        assert!(g2.is_ok());
        drop(g1);
    }
}
