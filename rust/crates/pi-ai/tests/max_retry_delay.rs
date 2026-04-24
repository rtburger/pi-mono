use futures::stream;
use parking_lot::Mutex;
use pi_ai::{
    AiProvider, AssistantEventStream, SimpleStreamOptions, StreamOptions, complete_simple,
    register_provider, unregister_provider,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    UserContent,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn base_context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

fn test_model(api: String) -> Model {
    Model {
        id: "capture-model".into(),
        name: "capture-model".into(),
        api,
        provider: "capture-provider".into(),
        base_url: "http://localhost".into(),
        reasoning: false,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 8_192,
        max_tokens: 2_048,
        compat: None,
    }
}

fn unique_test_api() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "test:max-retry-delay:{}",
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone)]
struct CaptureProvider {
    observed_max_retry_delays: Arc<Mutex<Vec<Option<u64>>>>,
}

impl AiProvider for CaptureProvider {
    fn stream(
        &self,
        model: Model,
        _context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        self.observed_max_retry_delays
            .lock()
            .push(options.max_retry_delay_ms);

        let mut message = AssistantMessage::empty(model.api, model.provider, model.id);
        message.content = vec![AssistantContent::Text {
            text: "ok".into(),
            text_signature: None,
        }];
        message.stop_reason = StopReason::Stop;

        Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message,
        })]))
    }
}

#[tokio::test]
async fn complete_simple_forwards_max_retry_delay_ms_to_registered_provider() {
    let api = unique_test_api();
    let observed_max_retry_delays = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        api.clone(),
        Arc::new(CaptureProvider {
            observed_max_retry_delays: observed_max_retry_delays.clone(),
        }),
    );

    let model = test_model(api.clone());

    complete_simple(
        model.clone(),
        base_context(),
        SimpleStreamOptions::default(),
    )
    .await
    .unwrap();

    complete_simple(
        model.clone(),
        base_context(),
        SimpleStreamOptions {
            max_retry_delay_ms: Some(4_321),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    complete_simple(
        model,
        base_context(),
        SimpleStreamOptions {
            max_retry_delay_ms: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    unregister_provider(&api);

    assert_eq!(
        observed_max_retry_delays.lock().clone(),
        vec![None, Some(4_321), Some(0)]
    );
}
