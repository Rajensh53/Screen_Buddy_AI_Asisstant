//! Turns a selection into an explanation: cleans the text, builds the prompt, and asks the
//! provider, remembering complete answers so the same text is not asked twice.

mod cache;
mod prompt;

pub use prompt::clean_input;

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::provider::{Provider, ProviderError};
use cache::LruCache;

/// Answers are short, so this stays well under 1 MB.
const CACHE_CAPACITY: usize = 200;

/// Cache key: model name and cleaned text.
type Key = (String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Explained {
    /// Streamed from the model just now.
    Fresh,
    /// Sent in one piece from the cache.
    Cached,
    /// Nothing readable was left after cleaning; the model was not asked.
    NothingToExplain,
}

pub struct Explainer<P> {
    provider: P,
    model: String,
    cache: Mutex<LruCache<Key, String>>,
}

impl<P: Provider> Explainer<P> {
    pub fn new(provider: P, model: impl Into<String>) -> Self {
        Self { provider, model: model.into(), cache: Mutex::new(LruCache::new(CACHE_CAPACITY)) }
    }

    /// Checks that the model can answer (server running, model installed).
    pub async fn health(&self) -> Result<(), ProviderError> {
        self.provider.health(&self.model).await
    }

    /// Explains `text`, passing the answer to `on_chunk` piece by piece.
    /// Only complete answers are cached: not errors, and not a request cancelled by dropping this future.
    pub async fn explain(
        &self,
        text: &str,
        mut on_chunk: impl FnMut(&str) + Send,
    ) -> Result<Explained, ProviderError> {
        let Some(text) = prompt::clean_input(text) else {
            return Ok(Explained::NothingToExplain);
        };
        let key = (self.model.clone(), text);

        // The lock is released at the end of this statement, before any await.
        let cached = self.cache().get(&key).cloned();
        if let Some(answer) = cached {
            on_chunk(&answer);
            return Ok(Explained::Cached);
        }

        let mut answer = String::new();
        let user = prompt::user_message(&key.1);
        let request = prompt::build_request(&self.model, &user);
        self.provider
            .stream_chat(request, |piece| {
                answer.push_str(piece);
                on_chunk(piece);
            })
            .await?;

        if !answer.trim().is_empty() {
            self.cache().put(key, answer);
        }
        Ok(Explained::Fresh)
    }

    fn cache(&self) -> MutexGuard<'_, LruCache<Key, String>> {
        // A panic while holding the lock cannot leave the cache half-updated, so keep using it.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::async_runtime::block_on;

    use super::*;
    use crate::provider::ChatRequest;

    /// Answers with fixed pieces (or a fixed error) and records what it was asked.
    struct FakeProvider {
        reply: Result<Vec<&'static str>, ProviderError>,
        calls: AtomicUsize,
        last_user_text: Mutex<String>,
    }

    impl FakeProvider {
        fn answering(pieces: &[&'static str]) -> Self {
            Self::with_reply(Ok(pieces.to_vec()))
        }

        fn with_reply(reply: Result<Vec<&'static str>, ProviderError>) -> Self {
            Self { reply, calls: AtomicUsize::new(0), last_user_text: Mutex::new(String::new()) }
        }
    }

    impl Provider for FakeProvider {
        async fn health(&self, _model: &str) -> Result<(), ProviderError> {
            Ok(())
        }

        async fn stream_chat(
            &self,
            req: ChatRequest<'_>,
            mut on_chunk: impl FnMut(&str) + Send,
        ) -> Result<(), ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_user_text.lock().unwrap() = req.user.to_owned();
            let pieces = self.reply.clone()?;
            for piece in pieces {
                on_chunk(piece);
            }
            Ok(())
        }
    }

    fn explain(explainer: &Explainer<FakeProvider>, text: &str) -> (Result<Explained, ProviderError>, Vec<String>) {
        let mut pieces = Vec::new();
        let result = block_on(explainer.explain(text, |p| pieces.push(p.to_owned())));
        (result, pieces)
    }

    fn calls(explainer: &Explainer<FakeProvider>) -> usize {
        explainer.provider.calls.load(Ordering::SeqCst)
    }

    #[test]
    fn pieces_are_forwarded_in_order() {
        let explainer = Explainer::new(FakeProvider::answering(&["Meaning: ", "luck"]), "m");
        let (result, pieces) = explain(&explainer, "serendipity");
        assert_eq!(result, Ok(Explained::Fresh));
        assert_eq!(pieces, ["Meaning: ", "luck"]);
    }

    #[test]
    fn same_text_again_comes_from_cache() {
        let explainer = Explainer::new(FakeProvider::answering(&["Meaning: ", "luck"]), "m");
        assert_eq!(explain(&explainer, "serendipity").0, Ok(Explained::Fresh));
        let (result, pieces) = explain(&explainer, "serendipity");
        assert_eq!(result, Ok(Explained::Cached));
        assert_eq!(pieces, ["Meaning: luck"]);
        assert_eq!(calls(&explainer), 1);
    }

    #[test]
    fn errors_are_not_cached() {
        let explainer = Explainer::new(FakeProvider::with_reply(Err(ProviderError::NotRunning)), "m");
        assert_eq!(explain(&explainer, "serendipity").0, Err(ProviderError::NotRunning));
        assert_eq!(explain(&explainer, "serendipity").0, Err(ProviderError::NotRunning));
        assert_eq!(calls(&explainer), 2);
    }

    #[test]
    fn empty_answers_are_not_cached() {
        let explainer = Explainer::new(FakeProvider::answering(&[" "]), "m");
        assert_eq!(explain(&explainer, "serendipity").0, Ok(Explained::Fresh));
        assert_eq!(explain(&explainer, "serendipity").0, Ok(Explained::Fresh));
        assert_eq!(calls(&explainer), 2);
    }

    #[test]
    fn nothing_to_explain_never_asks_the_model() {
        let explainer = Explainer::new(FakeProvider::answering(&["x"]), "m");
        let (result, pieces) = explain(&explainer, "\u{FFFC}\n \n");
        assert_eq!(result, Ok(Explained::NothingToExplain));
        assert!(pieces.is_empty());
        assert_eq!(calls(&explainer), 0);
    }

    #[test]
    fn text_is_cleaned_before_asking_and_caching() {
        let explainer = Explainer::new(FakeProvider::answering(&["answer"]), "m");
        assert_eq!(explain(&explainer, "\u{FFFC}word\r\n").0, Ok(Explained::Fresh));
        assert_eq!(*explainer.provider.last_user_text.lock().unwrap(), "<selected_text>\nword\n</selected_text>");
        assert_eq!(explain(&explainer, "word").0, Ok(Explained::Cached));
        assert_eq!(calls(&explainer), 1);
    }
}
