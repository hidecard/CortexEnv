use super::{ChatMessage, ChatModel, ChatRole, LlmError};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

/// OpenAI Chat Completions-compatible HTTP provider.
///
/// The client does not read environment variables or log credentials. Callers
/// provide the endpoint, key, and model explicitly so secret/configuration
/// management remains the responsibility of the host application.
pub struct OpenAiCompatibleClient {
    client: Client,
    endpoint: Url,
    api_key: String,
    model: String,
}

impl OpenAiCompatibleClient {
    /// Create a provider client from a versioned API base URL, API key, and model.
    ///
    /// For example, `https://api.openai.com/v1` is extended with
    /// `/chat/completions`. Local OpenAI-compatible servers may use an `http`
    /// URL; remote TLS endpoints should use `https`.
    pub fn new(
        base_url: &str,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, LlmError> {
        let mut endpoint = Url::parse(base_url).map_err(|_| LlmError::InvalidConfiguration)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(LlmError::InvalidConfiguration);
        }

        let api_key = api_key.into();
        let model = model.into();
        if api_key.trim().is_empty() || model.trim().is_empty() {
            return Err(LlmError::InvalidConfiguration);
        }

        let path = format!("{}/chat/completions", endpoint.path().trim_end_matches('/'));
        endpoint.set_path(&path);
        let client = Client::builder()
            .build()
            .map_err(|_| LlmError::InvalidConfiguration)?;

        Ok(Self {
            client,
            endpoint,
            api_key,
            model,
        })
    }
}

#[derive(Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
}

#[derive(Deserialize)]
struct CompletionMessage {
    content: Option<String>,
}

fn parse_completion(body: &[u8]) -> Result<ChatMessage, LlmError> {
    let response: CompletionResponse =
        serde_json::from_slice(body).map_err(|_| LlmError::InvalidResponse)?;
    let content = response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .filter(|content| !content.trim().is_empty())
        .ok_or(LlmError::InvalidResponse)?;
    Ok(ChatMessage {
        role: ChatRole::Assistant,
        content,
    })
}

#[async_trait]
impl ChatModel for OpenAiCompatibleClient {
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ChatMessage, LlmError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.api_key)
            .json(&CompletionRequest {
                model: &self.model,
                messages,
            })
            .send()
            .await
            .map_err(|_| LlmError::RequestFailed)?;

        if !response.status().is_success() {
            return Err(LlmError::HttpStatus(response.status().as_u16()));
        }

        let body = response
            .bytes()
            .await
            .map_err(|_| LlmError::InvalidResponse)?;
        parse_completion(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn appends_chat_completions_to_a_versioned_base_path() {
        let client =
            OpenAiCompatibleClient::new("https://example.test/v1/", "test-key", "test-model")
                .unwrap();
        assert_eq!(
            client.endpoint.as_str(),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn rejects_unsafe_or_incomplete_provider_configuration() {
        assert!(matches!(
            OpenAiCompatibleClient::new("file:///tmp/provider", "key", "model"),
            Err(LlmError::InvalidConfiguration)
        ));
        assert!(matches!(
            OpenAiCompatibleClient::new("https://example.test/v1", " ", "model"),
            Err(LlmError::InvalidConfiguration)
        ));
        assert!(matches!(
            OpenAiCompatibleClient::new("https://example.test/v1", "key", " "),
            Err(LlmError::InvalidConfiguration)
        ));
    }

    #[test]
    fn parses_assistant_text_and_rejects_empty_or_malformed_responses() {
        let parsed =
            parse_completion(br#"{"choices":[{"message":{"content":"fixed code"}}]}"#).unwrap();
        assert_eq!(parsed, ChatMessage::assistant("fixed code"));

        assert_eq!(
            parse_completion(br#"{"choices":[{"message":{"content":"  "}}]}"#),
            Err(LlmError::InvalidResponse)
        );
        assert_eq!(
            parse_completion(b"not json"),
            Err(LlmError::InvalidResponse)
        );
    }

    #[tokio::test]
    async fn sends_authenticated_openai_compatible_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            let (header_end, content_length) = loop {
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0, "client closed before sending request");
                request.extend_from_slice(&chunk[..count]);

                if let Some(header_end) = request.windows(4).position(|value| value == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .expect("JSON request has a content-length");
                    if request.len() >= header_end + 4 + content_length {
                        break (header_end, content_length);
                    }
                }
            };

            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            assert!(headers.starts_with("post /v1/chat/completions http/1.1"));
            assert!(headers.contains("authorization: bearer integration-key"));
            let body_start = header_end + 4;
            let payload: serde_json::Value =
                serde_json::from_slice(&request[body_start..body_start + content_length]).unwrap();
            assert_eq!(payload["model"], "integration-model");
            assert_eq!(payload["messages"][0]["role"], "user");
            assert_eq!(payload["messages"][0]["content"], "write safe code");

            let body = r#"{"choices":[{"message":{"content":"assistant reply"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = OpenAiCompatibleClient::new(
            &format!("http://{address}/v1"),
            "integration-key",
            "integration-model",
        )
        .unwrap();
        let answer = client
            .complete(&[ChatMessage::user("write safe code")])
            .await
            .unwrap();
        assert_eq!(answer, ChatMessage::assistant("assistant reply"));
        server.await.unwrap();
    }
}
