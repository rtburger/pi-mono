use parking_lot::Mutex;
use pi_agent::ThinkingLevel;
use pi_ai::{
    FauxModelDefinition, RegisterFauxProviderOptions, StreamOptions, register_faux_provider,
};
use pi_coding_agent_core::{
    AgentSessionOptions, CodingAgentCoreOptions, MemoryAuthStorage, SessionBootstrapOptions,
    SessionManager, create_agent_session,
};
use pi_events::{AssistantContent, Message, UserContent};
use std::{path::PathBuf, sync::Arc};

#[tokio::test]
async fn navigate_tree_restores_model_and_thinking_level_from_target_branch() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "tree-navigation-runtime-faux".into(),
        models: vec![
            FauxModelDefinition {
                id: "tree-navigation-runtime-faux-1".into(),
                name: Some("Tree Navigation Runtime Faux 1".into()),
                reasoning: true,
            },
            FauxModelDefinition {
                id: "tree-navigation-runtime-faux-2".into(),
                name: Some("Tree Navigation Runtime Faux 2".into()),
                reasoning: true,
            },
        ],
        ..RegisterFauxProviderOptions::default()
    });
    let model_one = faux
        .get_model(Some("tree-navigation-runtime-faux-1"))
        .expect("expected first faux model");
    let model_two = faux
        .get_model(Some("tree-navigation-runtime-faux-2"))
        .expect("expected second faux model");

    let cwd = PathBuf::from("/tmp/pi-tree-navigation-runtime");
    let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
    manager
        .append_model_change(model_one.provider.clone(), model_one.id.clone())
        .unwrap();
    manager.append_thinking_level_change("off").unwrap();
    let root_user_id = manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("root prompt"),
            }],
            timestamp: 1,
        })
        .unwrap();
    manager
        .append_message(Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("root answer"),
                text_signature: None,
            }],
            api: String::from("faux:test"),
            provider: model_one.provider.clone(),
            model: model_one.id.clone(),
            response_id: None,
            usage: Default::default(),
            stop_reason: pi_events::StopReason::Stop,
            error_message: None,
            timestamp: 2,
        })
        .unwrap();
    manager
        .append_model_change(model_two.provider.clone(), model_two.id.clone())
        .unwrap();
    manager.append_thinking_level_change("high").unwrap();
    manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("branch prompt"),
            }],
            timestamp: 3,
        })
        .unwrap();

    let created = create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model_one.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model_one.clone(), model_two.clone()],
            models_json_path: None,
            cwd: Some(cwd),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(Arc::new(Mutex::new(manager))),
    })
    .expect("expected agent session");
    let session = created.session;

    let initial_state = session.state();
    assert_eq!(initial_state.model.provider, model_two.provider);
    assert_eq!(initial_state.model.id, model_two.id);
    assert_eq!(initial_state.thinking_level, ThinkingLevel::High);

    let navigation = session
        .navigate_tree(Some(root_user_id.as_str()), Default::default())
        .await
        .expect("expected tree navigation to succeed");

    assert_eq!(navigation.editor_text.as_deref(), Some("root prompt"));

    let state = session.state();
    assert_eq!(state.model.provider, model_one.provider);
    assert_eq!(state.model.id, model_one.id);
    assert_eq!(state.thinking_level, ThinkingLevel::Off);
    assert!(state.messages.is_empty(), "messages: {:?}", state.messages);

    faux.unregister();
}
