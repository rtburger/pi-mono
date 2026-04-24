use crate::{
    models::{get_model_headers, get_provider_headers},
    partial_json,
};
use pi_events::{AssistantContent, AssistantEvent, AssistantMessage, Model, StopReason, Usage};
use reqwest::Client;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn shared_http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(Client::new)
}

pub(crate) fn build_runtime_request_headers(
    model: &Model,
    base_headers: BTreeMap<String, String>,
    option_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    merge_request_headers(
        base_headers,
        provider_request_headers(model),
        option_headers,
    )
}

pub(crate) fn merge_request_headers(
    mut headers: BTreeMap<String, String>,
    inherited_headers: BTreeMap<String, String>,
    option_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    headers.extend(inherited_headers);
    headers.extend(option_headers.clone());
    headers
}

pub(crate) fn provider_request_headers(model: &Model) -> BTreeMap<String, String> {
    get_model_headers(&model.provider, &model.id)
        .or_else(|| get_provider_headers(&model.provider))
        .unwrap_or_default()
}

pub(crate) fn terminal_error_message(
    model: &Model,
    error_message: impl Into<String>,
) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: Vec::new(),
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::Error,
        error_message: Some(error_message.into()),
        timestamp: now_ms(),
    }
}

pub(crate) fn assistant_text_content(output: &AssistantMessage, index: usize) -> Option<String> {
    match output.content.get(index) {
        Some(AssistantContent::Text { text, .. }) => Some(text.clone()),
        _ => None,
    }
}

pub(crate) fn assistant_thinking_content(
    output: &AssistantMessage,
    index: usize,
) -> Option<String> {
    match output.content.get(index) {
        Some(AssistantContent::Thinking { thinking, .. }) => Some(thinking.clone()),
        _ => None,
    }
}

pub(crate) fn parse_streaming_json_map(input: &str) -> BTreeMap<String, Value> {
    partial_json::parse_partial_json_map(input)
}

pub(crate) fn is_terminal_event(event: &AssistantEvent) -> bool {
    matches!(
        event,
        AssistantEvent::Done { .. } | AssistantEvent::Error { .. }
    )
}

pub(crate) fn is_signal_aborted(signal: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

pub(crate) async fn wait_for_abort(signal: &mut tokio::sync::watch::Receiver<bool>) {
    while !*signal.borrow() {
        if signal.changed().await.is_err() {
            return;
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{build_runtime_request_headers, merge_request_headers, shared_http_client};
    use pi_events::{Model, ModelCost};
    use std::collections::BTreeMap;

    fn model() -> Model {
        Model {
            id: "test-model".into(),
            name: "test-model".into(),
            api: "test-api".into(),
            provider: "test-provider".into(),
            base_url: "https://example.invalid".into(),
            reasoning: false,
            input: vec!["text".into()],
            cost: ModelCost::default(),
            context_window: 1,
            max_tokens: 1,
            compat: None,
        }
    }

    #[test]
    fn shared_http_client_returns_same_instance() {
        assert!(std::ptr::eq(shared_http_client(), shared_http_client()));
    }

    #[test]
    fn merge_request_headers_applies_base_then_inherited_then_options() {
        let headers = merge_request_headers(
            BTreeMap::from([
                ("accept".into(), "text/event-stream".into()),
                ("x-order".into(), "base".into()),
            ]),
            BTreeMap::from([
                ("x-inherited".into(), "provider".into()),
                ("x-order".into(), "inherited".into()),
            ]),
            &BTreeMap::from([
                ("x-option".into(), "runtime".into()),
                ("x-order".into(), "option".into()),
            ]),
        );

        assert_eq!(
            headers.get("accept").map(String::as_str),
            Some("text/event-stream")
        );
        assert_eq!(
            headers.get("x-inherited").map(String::as_str),
            Some("provider")
        );
        assert_eq!(headers.get("x-option").map(String::as_str), Some("runtime"));
        assert_eq!(headers.get("x-order").map(String::as_str), Some("option"));
    }

    #[test]
    fn build_runtime_request_headers_preserves_base_headers_without_catalog_entries() {
        let headers = build_runtime_request_headers(
            &model(),
            BTreeMap::from([("accept".into(), "text/event-stream".into())]),
            &BTreeMap::from([("x-runtime".into(), "1".into())]),
        );

        assert_eq!(
            headers.get("accept").map(String::as_str),
            Some("text/event-stream")
        );
        assert_eq!(headers.get("x-runtime").map(String::as_str), Some("1"));
    }
}
