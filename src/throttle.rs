//! Per-engine concurrency throttle for parallel search.
//!
//! Two-level model:
//! - **Across engines**: always concurrent (a `JoinSet` fans out).
//! - **Within one engine family**: bounded by a semaphore.
//!
//! Families (see AGENTS.md for why):
//! - `sogou` + `weixin` share one backend trust bucket (Sogou IP风控 +
//!   shared SNUID cookie). Permits = 1: searches serialize end-to-end,
//!   including multi-page delays. This also papers over the
//!   check-then-sleep race in `sogou.rs::polite_wait`.
//! - `dblp`, `startpage`: Anubis PoW solve is full-core CPU work; permits = 2
//!   keeps two concurrent solves from starving the machine while still
//!   allowing parallelism once the week-long auth cookie is cached.
//! - everything else (exa, bing, csdn, juejin, cnki, ieee, deepseek-official):
//!   stateless per-request clients, no local shared state → bounded only by
//!   the global cap.
//! - `deepseek-go` spawns a `python.exe` process per query (see
//!   `deepseek.rs`); it has its own process semaphore there.
//!
//! A global semaphore caps total concurrent engine calls so that
//! `N queries × M engines` from one batched `web` call cannot open hundreds of
//! connections through the proxy at once.

use std::sync::OnceLock;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Sogou + weixin share one permit: they are the same backend trust domain.
static SOGOU_FAMILY_SEM: OnceLock<std::sync::Arc<Semaphore>> = OnceLock::new();
static DBLP_SEM: OnceLock<std::sync::Arc<Semaphore>> = OnceLock::new();
static STARTPAGE_SEM: OnceLock<std::sync::Arc<Semaphore>> = OnceLock::new();
/// Global cap on concurrent engine calls across all queries.
static GLOBAL_SEM: OnceLock<std::sync::Arc<Semaphore>> = OnceLock::new();

fn sogou_family_sem() -> std::sync::Arc<Semaphore> {
    SOGOU_FAMILY_SEM
        .get_or_init(|| std::sync::Arc::new(Semaphore::new(1)))
        .clone()
}

fn dblp_sem() -> std::sync::Arc<Semaphore> {
    DBLP_SEM
        .get_or_init(|| std::sync::Arc::new(Semaphore::new(2)))
        .clone()
}

fn startpage_sem() -> std::sync::Arc<Semaphore> {
    STARTPAGE_SEM
        .get_or_init(|| std::sync::Arc::new(Semaphore::new(2)))
        .clone()
}

fn global_sem() -> std::sync::Arc<Semaphore> {
    GLOBAL_SEM
        .get_or_init(|| std::sync::Arc::new(Semaphore::new(32)))
        .clone()
}

/// True for engines that share Sogou's backend trust bucket.
pub fn is_sogou_family(engine: &str) -> bool {
    matches!(engine, "sogou" | "weixin")
}

/// Acquire all permits needed to run `engine`. The returned guards must be
/// held for the whole `engine.search(...)` call (including pagination).
/// Permits are acquired in a fixed order (family first, global last) so
/// concurrent acquirers cannot deadlock.
pub async fn acquire(engine: &str) -> Vec<OwnedSemaphorePermit> {
    let mut permits = Vec::with_capacity(2);
    let family = match engine {
        e if is_sogou_family(e) => Some(sogou_family_sem()),
        "dblp" => Some(dblp_sem()),
        "startpage" => Some(startpage_sem()),
        _ => None,
    };
    if let Some(sem) = family {
        permits.push(
            sem.acquire_owned()
                .await
                .expect("engine family semaphore closed"),
        );
    }
    permits.push(
        global_sem()
            .acquire_owned()
            .await
            .expect("global search semaphore closed"),
    );
    permits
}
