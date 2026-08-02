//! Warehouse interfaces.
//!
//! Two stores, because the data has two shapes. Structured facts — views,
//! cut times, loudness, drop spans — belong in a relational table and are
//! queried by id and range. Meaning — what a hook promises, what a cutaway is
//! *of* — cannot be queried that way, so it goes to a vector index and is
//! retrieved by similarity.
//!
//! Both are traits so the mock here can be swapped for Postgres and Qdrant
//! without the diff engine noticing. Methods take `&self`: a real store hands
//! out connections from a pool, and the UI holds one handle across threads.

use crate::ai_tooling::competitor::models::CompetitorVideo;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("no competitor video with id {0}")]
    NotFound(String),

    #[error("store unavailable: {0}")]
    Backend(String),

    #[error("embedding failed: {0}")]
    Embedding(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Structured competitor facts. The relational half of the warehouse.
///
/// `Send + Sync` because the UI holds one handle and background tasks use it.
/// The `impl Future + Send` returns are spelled out rather than written as
/// `async fn` so the futures are guaranteed spawnable.
pub trait CompetitorDataStore: Send + Sync {
    /// Upserts by `video_id` — re-analysing a video replaces its row.
    fn save_video(&self, video: CompetitorVideo) -> impl Future<Output = Result<()>> + Send;

    fn load_video(&self, video_id: &str)
        -> impl Future<Output = Result<CompetitorVideo>> + Send;

    /// Every analysed video for a channel, most viral first.
    fn videos_for_channel(
        &self,
        channel_id: &str,
    ) -> impl Future<Output = Result<Vec<CompetitorVideo>>> + Send;

    /// Ids already deconstructed, so ingestion can skip them.
    fn known_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send;
}

/// Semantic retrieval. The vector half of the warehouse.
///
/// What gets embedded is deliberately narrow: hook promises and B-roll topics.
/// Those are the two things the diff engine matches *by meaning* — "the user is
/// talking about latency here, the competitor cut to a stopwatch shot at the
/// same narrative beat" is a similarity question, not a SQL one.
pub trait SemanticIndex: Send + Sync {
    /// Indexes one passage under a video and a kind.
    fn index(
        &self,
        entry: SemanticEntry,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Nearest passages to `query`, best first.
    fn search(
        &self,
        query: &str,
        kind: SemanticKind,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<SemanticHit>>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticKind {
    /// What a hook promises the viewer.
    HookPromise,
    /// What a cutaway shows.
    BRollTopic,
}

#[derive(Debug, Clone)]
pub struct SemanticEntry {
    pub video_id: String,
    pub kind: SemanticKind,
    /// The passage itself; the backend embeds this.
    pub text: String,
    /// Where in the source video it sits.
    pub timestamp_sec: f32,
}

#[derive(Debug, Clone)]
pub struct SemanticHit {
    pub entry: SemanticEntry,
    /// Cosine similarity, 0.0..=1.0.
    pub score: f32,
}

// ------------------------------------------------------------------ mock

/// In-memory warehouse standing in for Postgres + a vector database.
///
/// Cloning shares the data — it is one warehouse behind an `Arc`, not a copy
/// per holder, which is what lets the UI and a background task both write.
#[derive(Clone, Default)]
pub struct InMemoryWarehouse {
    videos: Arc<Mutex<HashMap<String, CompetitorVideo>>>,
    vectors: Arc<Mutex<Vec<SemanticEntry>>>,
}

impl InMemoryWarehouse {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.videos.lock().map(|v| v.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned mutex means another thread panicked mid-write. The data is
    /// still readable and this is a cache, so recover rather than propagate.
    fn videos(&self) -> std::sync::MutexGuard<'_, HashMap<String, CompetitorVideo>> {
        self.videos.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn vectors(&self) -> std::sync::MutexGuard<'_, Vec<SemanticEntry>> {
        self.vectors.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl CompetitorDataStore for InMemoryWarehouse {
    async fn save_video(&self, video: CompetitorVideo) -> Result<()> {
        // A real backend round-trips; the delay keeps the UI honest about it.
        simulate_io(40).await;
        self.videos().insert(video.video_id.clone(), video);
        Ok(())
    }

    async fn load_video(&self, video_id: &str) -> Result<CompetitorVideo> {
        simulate_io(25).await;
        self.videos()
            .get(video_id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(video_id.to_string()))
    }

    async fn videos_for_channel(&self, channel_id: &str) -> Result<Vec<CompetitorVideo>> {
        simulate_io(60).await;
        let mut rows: Vec<CompetitorVideo> = self
            .videos()
            .values()
            .filter(|video| video.channel_id == channel_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.outlier_multiplier.total_cmp(&a.outlier_multiplier));
        Ok(rows)
    }

    async fn known_ids(&self) -> Result<Vec<String>> {
        simulate_io(10).await;
        Ok(self.videos().keys().cloned().collect())
    }
}

impl SemanticIndex for InMemoryWarehouse {
    async fn index(&self, entry: SemanticEntry) -> Result<()> {
        simulate_io(30).await;
        self.vectors().push(entry);
        Ok(())
    }

    async fn search(
        &self,
        query: &str,
        kind: SemanticKind,
        limit: usize,
    ) -> Result<Vec<SemanticHit>> {
        simulate_io(80).await;

        let mut hits: Vec<SemanticHit> = self
            .vectors()
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| SemanticHit {
                score: lexical_similarity(query, &entry.text),
                entry: entry.clone(),
            })
            .filter(|hit| hit.score > 0.0)
            .collect();

        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Stands in for a network round trip.
async fn simulate_io(millis: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
}

/// Jaccard overlap on lowercased words.
///
/// A stand-in for cosine distance over real embeddings: same shape (0.0..=1.0,
/// higher is closer), no model. It matches shared vocabulary, not meaning, so
/// swapping in a real index changes the *quality* of the diff, not its logic.
fn lexical_similarity(left: &str, right: &str) -> f32 {
    let tokens = |text: &str| -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.len() > 2)
            .map(str::to_string)
            .collect()
    };

    let (a, b) = (tokens(left), tokens(right));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let shared = a.iter().filter(|word| b.contains(word)).count();
    let union = a.len() + b.len() - shared;
    if union == 0 {
        return 0.0;
    }
    shared as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::competitor::models::*;

    fn video(id: &str, channel: &str, multiplier: f64) -> CompetitorVideo {
        CompetitorVideo {
            video_id: id.into(),
            channel_id: channel.into(),
            title: format!("video {id}"),
            view_count: 1000,
            outlier_multiplier: multiplier,
            duration_sec: 300.0,
            published_at: None,
            retention: RetentionAnalysis::default(),
            audio: AudioDynamics::default(),
            structure: VisualAndPacingStructure::default(),
            transcript: TranscriptAndHooks::default(),
            analyzed_at: 0,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    #[test]
    fn saving_the_same_id_twice_replaces_it_rather_than_duplicating() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            store.save_video(video("a", "UC1", 2.0)).await.expect("save");
            store.save_video(video("a", "UC1", 5.0)).await.expect("save");

            assert_eq!(store.len(), 1);
            let loaded = store.load_video("a").await.expect("load");
            assert_eq!(loaded.outlier_multiplier, 5.0, "the later analysis wins");
        });
    }

    #[test]
    fn a_channel_query_returns_only_its_videos_most_viral_first() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            store.save_video(video("a", "UC1", 2.0)).await.expect("save");
            store.save_video(video("b", "UC1", 7.0)).await.expect("save");
            store.save_video(video("c", "UC2", 9.0)).await.expect("save");

            let rows = store.videos_for_channel("UC1").await.expect("query");
            let ids: Vec<&str> = rows.iter().map(|v| v.video_id.as_str()).collect();
            assert_eq!(ids, vec!["b", "a"]);
        });
    }

    #[test]
    fn a_missing_row_is_an_error_not_an_empty_video() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            let err = store.load_video("nope").await.expect_err("should fail");
            assert!(matches!(err, StoreError::NotFound(id) if id == "nope"));
        });
    }

    #[test]
    fn the_vector_index_retrieves_by_overlap_and_respects_its_kind() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            store
                .index(SemanticEntry {
                    video_id: "a".into(),
                    kind: SemanticKind::BRollTopic,
                    text: "slow motion coffee pour in a dark kitchen".into(),
                    timestamp_sec: 12.0,
                })
                .await
                .expect("index");
            store
                .index(SemanticEntry {
                    video_id: "a".into(),
                    kind: SemanticKind::HookPromise,
                    text: "promises to show a coffee brewing trick".into(),
                    timestamp_sec: 0.0,
                })
                .await
                .expect("index");

            let hits = store
                .search("coffee pour kitchen", SemanticKind::BRollTopic, 5)
                .await
                .expect("search");

            assert_eq!(hits.len(), 1, "the hook entry is a different kind");
            assert!(hits[0].score > 0.0);
            assert_eq!(hits[0].entry.timestamp_sec, 12.0);
        });
    }

    #[test]
    fn an_unrelated_query_returns_nothing_rather_than_the_whole_index() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            store
                .index(SemanticEntry {
                    video_id: "a".into(),
                    kind: SemanticKind::BRollTopic,
                    text: "aerial drone shot of mountains".into(),
                    timestamp_sec: 5.0,
                })
                .await
                .expect("index");

            let hits = store
                .search("keyboard closeup", SemanticKind::BRollTopic, 5)
                .await
                .expect("search");
            assert!(hits.is_empty());
        });
    }

    #[test]
    fn the_warehouse_is_shared_by_every_clone_not_copied() {
        runtime().block_on(async {
            let store = InMemoryWarehouse::new();
            let handle = store.clone();
            handle.save_video(video("a", "UC1", 2.0)).await.expect("save");

            assert_eq!(store.len(), 1, "the write is visible through the original");
        });
    }

    #[test]
    fn similarity_is_bounded_and_symmetric() {
        assert_eq!(lexical_similarity("coffee pour", "coffee pour"), 1.0);
        assert_eq!(lexical_similarity("", "anything"), 0.0);
        assert_eq!(
            lexical_similarity("dark kitchen", "kitchen dark"),
            lexical_similarity("kitchen dark", "dark kitchen")
        );
    }
}
