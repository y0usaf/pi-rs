//! Cancellation — the Rust shape of the spec's `AbortSignal`.
//!
//! Spec: `utils/abort-signals.ts` plus the ambient DOM
//! `AbortController`/`AbortSignal` pair. One cloneable handle replaces
//! the controller/signal split (mechanism detail); `combineAbortSignals`
//! becomes `tokio::select!` over [`AbortSignal::aborted`] futures at the
//! await sites that need it.

use tokio::sync::watch;

/// Cloneable cancellation handle. All clones observe the same abort flag;
/// any clone may trigger it. Aborting is idempotent and sticky.
#[derive(Clone, Debug)]
pub struct AbortSignal {
    tx: watch::Sender<bool>,
}

impl AbortSignal {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    /// Trigger the abort. Spec: `AbortController.abort()`.
    pub fn abort(&self) {
        self.tx.send_replace(true);
    }

    /// Spec: `signal.aborted`.
    pub fn is_aborted(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves when the signal is aborted; pends forever otherwise.
    /// Spec: `signal.addEventListener("abort", …)`.
    pub async fn aborted(&self) {
        let mut rx = self.tx.subscribe();
        if rx.wait_for(|aborted| *aborted).await.is_err() {
            // The sender lives in `self`, so the channel cannot close
            // while we hold `&self`; never fabricate an abort.
            std::future::pending::<()>().await;
        }
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}
