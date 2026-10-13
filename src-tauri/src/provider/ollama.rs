//! Streaming client for the local Ollama server.
//!
//! Selected text is only ever sent to `BASE_URL` (hard rule), never through a proxy and never
//! to a redirect target. API reference: https://github.com/ollama/ollama/blob/main/docs/api.md

use std::time::Duration;

use reqwest::{Client, Response, redirect};
use serde::{Deserialize, Serialize};

use super::{ChatRequest, Provider, ProviderError};

/// The only address selected text is sent to. Not configurable.
const BASE_URL: &str = "http://127.0.0.1:11434";

/// Localhost answers at once; a closed port should fail fast.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest gap between two pieces of a streamed answer. Allows a cold model load.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

// Model options from the project brief.
const TEMPERATURE: f64 = 0.2;
const KEEP_ALIVE: &str = "30m";

pub struct OllamaProvider {
    client: Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new() -> reqwest::Result<Self> {
        Self::with_base_url(BASE_URL)
    }

    /// Private: the app always uses `BASE_URL`; tests point this at a fake server.
    fn with_base_url(base_url: &str) -> reqwest::Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .redirect(redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()?;
        Ok(Self { client, base_url: base_url.to_owned() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

impl Provider for OllamaProvider {
    /// `GET /api/tags` lists the installed models.
    async fn health(&self, model: &str) -> Result<(), ProviderError> {
        let response =
            self.client.get(self.url("/api/tags")).timeout(HEALTH_TIMEOUT).send().await.map_err(from_reqwest)?;
        if !response.status().is_success() {
            return Err(status_error(response, model).await);
        }
        let body = response.bytes().await.map_err(from_reqwest)?;
        let tags: Tags = serde_json::from_slice(&body).map_err(|_| ProviderError::BadResponse)?;
        if model_installed(tags.models.iter().map(|m| m.name.as_str()), model) {
            Ok(())
        } else {
            Err(ProviderError::ModelMissing(model.to_owned()))
        }
    }

    /// `POST /api/chat` with `stream: true` answers with one JSON object per line.
    async fn stream_chat(
        &self,
        req: ChatRequest<'_>,
        mut on_chunk: impl FnMut(&str) + Send,
    ) -> Result<(), ProviderError> {
        let body = ChatBody::new(req);
        let mut response =
            self.client.post(self.url("/api/chat")).json(&body).send().await.map_err(from_reqwest)?;
        if !response.status().is_success() {
            return Err(status_error(response, req.model).await);
        }

        let mut lines = LineBuffer::default();
        while let Some(bytes) = response.chunk().await.map_err(from_reqwest)? {
            for line in lines.push(&bytes) {
                if handle_line(&line, req.model, &mut on_chunk)? {
                    return Ok(());
                }
            }
        }
        if let Some(line) = lines.finish()
            && handle_line(&line, req.model, &mut on_chunk)?
        {
            return Ok(());
        }
        // The stream ended before Ollama said `done`: the answer was cut off.
        Err(ProviderError::BadResponse)
    }
}

#[derive(Serialize)]
struct ChatBody<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    stream: bool,
    /// Thinking off: faster, and the popup only needs the answer.
    think: bool,
    keep_alive: &'a str,
    options: Options,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct Options {
    temperature: f64,
}

impl<'a> ChatBody<'a> {
    fn new(req: ChatRequest<'a>) -> Self {
        Self {
            model: req.model,
            messages: [
                Message { role: "system", content: req.system },
                Message { role: "user", content: req.user },
            ],
            stream: true,
            think: false,
            keep_alive: KEEP_ALIVE,
            options: Options { temperature: TEMPERATURE },
        }
    }
}

/// One line of a streamed `/api/chat` answer.
#[derive(Deserialize)]
struct StreamLine {
    message: Option<LineMessage>,
    #[serde(default)]
    done: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct LineMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// Handles one streamed line. Returns `true` when the answer is complete.
fn handle_line(line: &[u8], model: &str, on_chunk: &mut impl FnMut(&str)) -> Result<bool, ProviderError> {
    let line: StreamLine = serde_json::from_slice(line).map_err(|_| ProviderError::BadResponse)?;
    if let Some(msg) = line.error {
        return Err(ollama_error(msg, model));
    }
    if let Some(message) = &line.message
        && !message.content.is_empty()
    {
        on_chunk(&message.content);
    }
    Ok(line.done)
}

/// Turns a non-2xx response into an error, using Ollama's `{"error": "..."}` body when present.
async fn status_error(response: Response, model: &str) -> ProviderError {
    let status = response.status();
    let msg = match response.bytes().await {
        Ok(body) => serde_json::from_slice::<ErrorBody>(&body).map(|b| b.error).ok(),
        Err(_) => None,
    };
    ollama_error(msg.unwrap_or_else(|| status.to_string()), model)
}

/// Ollama says `model "x" not found, try pulling it first` for a missing model.
fn ollama_error(msg: String, model: &str) -> ProviderError {
    let lower = msg.to_lowercase();
    if lower.contains("model") && lower.contains("not found") {
        ProviderError::ModelMissing(model.to_owned())
    } else {
        ProviderError::Ollama(msg)
    }
}

fn from_reqwest(err: reqwest::Error) -> ProviderError {
    if err.is_connect() {
        ProviderError::NotRunning
    } else if err.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Ollama(err.without_url().to_string())
    }
}

/// Whether `wanted` is in the installed list. A name without a tag means `:latest`, as in Ollama.
fn model_installed<'a>(installed: impl IntoIterator<Item = &'a str>, wanted: &str) -> bool {
    let wanted = full_model_name(wanted);
    installed.into_iter().any(|name| full_model_name(name) == wanted)
}

fn full_model_name(name: &str) -> String {
    let name = name.trim().to_lowercase();
    // The tag follows the last ':' of the last path part (`hf.co/user/repo:tag`).
    let last_part = name.rsplit('/').next().unwrap_or(&name);
    if last_part.contains(':') { name } else { format!("{name}:latest") }
}

/// Splits a byte stream into lines. Keeps an unfinished line (which may end in half of a
/// UTF-8 character) until the rest arrives.
#[derive(Default)]
struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    /// Adds bytes and returns the complete, non-blank lines without their line ending.
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.pending.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            if let Some(line) = non_blank(&line) {
                lines.push(line);
            }
        }
        lines
    }

    /// The last line, if the stream did not end with a line break.
    fn finish(self) -> Option<Vec<u8>> {
        non_blank(&self.pending)
    }
}

fn non_blank(line: &[u8]) -> Option<Vec<u8>> {
    let line = line.trim_ascii();
    (!line.is_empty()).then(|| line.to_vec())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use serde_json::json;
    use tauri::async_runtime::block_on;

    use super::*;

    const MODEL: &str = "qwen3.5:4b";

    fn request() -> ChatRequest<'static> {
        ChatRequest { model: MODEL, system: "system prompt", user: "serendipity" }
    }

    fn texts(lines: Vec<Vec<u8>>) -> Vec<String> {
        lines.into_iter().map(|l| String::from_utf8(l).unwrap()).collect()
    }

    // --- Fixed endpoint ---

    #[test]
    fn base_url_is_local_ollama_only() {
        assert_eq!(BASE_URL, "http://127.0.0.1:11434");
        assert_eq!(OllamaProvider::new().unwrap().base_url, BASE_URL);
    }

    // --- LineBuffer ---

    #[test]
    fn line_split_over_two_chunks() {
        let mut buf = LineBuffer::default();
        assert!(buf.push(b"{\"a\":").is_empty());
        assert_eq!(texts(buf.push(b"1}\n")), ["{\"a\":1}"]);
    }

    #[test]
    fn several_lines_in_one_chunk() {
        let mut buf = LineBuffer::default();
        assert_eq!(texts(buf.push(b"one\ntwo\r\nthree")), ["one", "two"]);
        assert_eq!(texts(buf.finish().into_iter().collect()), ["three"]);
    }

    #[test]
    fn urdu_character_split_across_chunks() {
        let line = "سلام\n".as_bytes();
        // Split inside the first two-byte character.
        let mut buf = LineBuffer::default();
        assert!(buf.push(&line[..1]).is_empty());
        assert_eq!(texts(buf.push(&line[1..])), ["سلام"]);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let mut buf = LineBuffer::default();
        assert_eq!(texts(buf.push(b"\n  \r\nx\n\n")), ["x"]);
        assert!(buf.finish().is_none());
    }

    // --- Model names ---

    #[test]
    fn model_installed_exact_and_latest() {
        assert!(model_installed(["qwen3.5:4b", "llama3:latest"], "qwen3.5:4b"));
        assert!(model_installed(["llama3:latest"], "llama3"));
        assert!(model_installed(["llama3"], "llama3:latest"));
        assert!(model_installed(["Qwen3.5:4B"], "qwen3.5:4b"));
    }

    #[test]
    fn model_installed_rejects_other_tags_and_empty_list() {
        assert!(!model_installed(["qwen3.5:9b"], "qwen3.5:4b"));
        assert!(!model_installed(["qwen3.5:4b"], "qwen3.5"));
        assert!(!model_installed([], "qwen3.5:4b"));
        assert!(model_installed(["hf.co/user/repo:q4"], "hf.co/user/repo:q4"));
    }

    // --- Request and response shapes ---

    #[test]
    fn chat_body_has_the_brief_options() {
        let body = serde_json::to_value(ChatBody::new(request())).unwrap();
        assert_eq!(
            body,
            json!({
                "model": MODEL,
                "messages": [
                    {"role": "system", "content": "system prompt"},
                    {"role": "user", "content": "serendipity"},
                ],
                "stream": true,
                "think": false,
                "keep_alive": "30m",
                "options": {"temperature": 0.2},
            })
        );
    }

    #[test]
    fn stream_line_content_and_done() {
        let mut got = String::new();
        let mut push = |s: &str| got.push_str(s);
        assert!(!handle_line(br#"{"message":{"role":"assistant","content":"Hi"},"done":false}"#, MODEL, &mut push).unwrap());
        assert!(handle_line(br#"{"message":{"role":"assistant","content":""},"done":true,"total_duration":5}"#, MODEL, &mut push).unwrap());
        assert_eq!(got, "Hi");
    }

    #[test]
    fn stream_line_errors() {
        let mut ignore = |_: &str| {};
        assert_eq!(
            handle_line(br#"{"error":"out of memory"}"#, MODEL, &mut ignore),
            Err(ProviderError::Ollama("out of memory".into()))
        );
        assert_eq!(
            handle_line(br#"{"error":"model \"x\" not found, try pulling it first"}"#, MODEL, &mut ignore),
            Err(ProviderError::ModelMissing(MODEL.into()))
        );
        assert_eq!(handle_line(b"not json", MODEL, &mut ignore), Err(ProviderError::BadResponse));
    }

    // --- Against a fake server ---

    /// Serves one connection: reads the whole request, then writes `parts` one by one.
    fn serve_once(parts: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            for part in parts {
                stream.write_all(&part).unwrap();
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(10));
            }
        });
        format!("http://{addr}")
    }

    fn read_request(stream: &mut TcpStream) {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                let len = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                if data.len() >= end + 4 + len {
                    return;
                }
            }
        }
    }

    /// A body without a length, ended by closing the connection.
    fn ok_head() -> Vec<u8> {
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n".to_vec()
    }

    fn json_response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn chat(base_url: &str) -> (Result<(), ProviderError>, String) {
        let provider = OllamaProvider::with_base_url(base_url).unwrap();
        let mut answer = String::new();
        let result = block_on(provider.stream_chat(request(), |piece| answer.push_str(piece)));
        (result, answer)
    }

    #[test]
    fn closed_port_means_not_running() {
        let addr = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        // The listener is dropped, so nothing listens on `addr` any more.
        let (result, _) = chat(&format!("http://{addr}"));
        assert_eq!(result, Err(ProviderError::NotRunning));
    }

    #[test]
    fn streamed_answer_in_odd_chunks_is_joined() {
        let body = concat!(
            r#"{"message":{"role":"assistant","content":"Meaning: "},"done":false}"#, "\n",
            r#"{"message":{"role":"assistant","content":"خوش قسمتی"},"done":false}"#, "\n",
            r#"{"message":{"role":"assistant","content":""},"done":true}"#, "\n",
        )
        .as_bytes();
        // Cut inside JSON and inside an Urdu character.
        let urdu_at = body.windows(2).position(|w| w == "خ".as_bytes()).unwrap();
        let cuts = [7, urdu_at + 1, body.len() - 3];
        let mut parts = vec![ok_head()];
        let mut start = 0;
        for cut in cuts {
            parts.push(body[start..cut].to_vec());
            start = cut;
        }
        parts.push(body[start..].to_vec());

        let (result, answer) = chat(&serve_once(parts));
        assert_eq!(result, Ok(()));
        assert_eq!(answer, "Meaning: خوش قسمتی");
    }

    #[test]
    fn answer_cut_off_before_done_is_bad_response() {
        let parts = vec![ok_head(), br#"{"message":{"content":"Mean"},"done":false}"#.to_vec()];
        let (result, answer) = chat(&serve_once(parts));
        assert_eq!(result, Err(ProviderError::BadResponse));
        assert_eq!(answer, "Mean");
    }

    #[test]
    fn missing_model_404() {
        let body = r#"{"error":"model \"qwen3.5:4b\" not found, try pulling it first"}"#;
        let (result, _) = chat(&serve_once(vec![json_response("404 Not Found", body)]));
        assert_eq!(result, Err(ProviderError::ModelMissing(MODEL.into())));
    }

    #[test]
    fn error_in_the_middle_of_the_stream() {
        let parts = vec![
            ok_head(),
            b"{\"message\":{\"content\":\"Hi\"},\"done\":false}\n".to_vec(),
            b"{\"error\":\"model runner has unexpectedly stopped\"}\n".to_vec(),
        ];
        let (result, _) = chat(&serve_once(parts));
        assert_eq!(result, Err(ProviderError::Ollama("model runner has unexpectedly stopped".into())));
    }

    #[test]
    fn health_checks_the_installed_models() {
        let tags = r#"{"models":[{"name":"qwen3.5:4b","model":"qwen3.5:4b","size":1}]}"#;
        let base = serve_once(vec![json_response("200 OK", tags)]);
        let provider = OllamaProvider::with_base_url(&base).unwrap();
        assert_eq!(block_on(provider.health(MODEL)), Ok(()));

        let base = serve_once(vec![json_response("200 OK", tags)]);
        let provider = OllamaProvider::with_base_url(&base).unwrap();
        assert_eq!(block_on(provider.health("llama3")), Err(ProviderError::ModelMissing("llama3".into())));
    }
}
