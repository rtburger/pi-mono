use pi_ai::{StreamOptions, register_faux_provider};
use pi_coding_agent_core::{
    AgentSessionOptions, AgentSessionRuntimeError, AgentSessionRuntimeRequest,
    CodingAgentCoreOptions, CreateAgentSessionRuntimeFactory, MemoryAuthStorage,
    SessionBootstrapOptions, SessionCwdIssue, SessionManager, CURRENT_SESSION_VERSION,
    assert_session_cwd_exists, create_agent_session, create_agent_session_runtime,
    get_missing_session_cwd_issue,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_session_file(path: &Path, cwd: &Path) {
    fs::write(
        path,
        format!(
            concat!(
                "{{\"type\":\"session\",",
                "\"version\":{},",
                "\"id\":\"session-id\",",
                "\"timestamp\":\"2026-01-01T00:00:00.000Z\",",
                "\"cwd\":{}}}\n"
            ),
            CURRENT_SESSION_VERSION,
            serde_json::to_string(&cwd.to_string_lossy().to_string()).unwrap(),
        ),
    )
    .unwrap();
}

fn build_runtime_factory(
    cwd: &Path,
) -> (CreateAgentSessionRuntimeFactory, impl FnOnce()) {
    let faux = register_faux_provider(Default::default());
    let model = faux.get_model(None).expect("expected faux model");
    let auth_source = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        String::from("faux-key"),
    )]));

    let factory: CreateAgentSessionRuntimeFactory = Arc::new(move |request| {
        let model = model.clone();
        let auth_source = auth_source.clone();
        Box::pin(async move {
            create_agent_session(AgentSessionOptions {
                core: CodingAgentCoreOptions {
                    auth_source,
                    built_in_models: vec![model.clone()],
                    models_json_path: None,
                    cwd: Some(request.cwd.clone()),
                    tools: Some(Vec::new()),
                    system_prompt: String::new(),
                    bootstrap: SessionBootstrapOptions::default(),
                    stream_options: StreamOptions::default(),
                },
                session_manager: request.session_manager,
            })
            .map_err(Into::into)
        })
    });

    let cleanup_cwd = cwd.to_path_buf();
    (
        factory,
        move || {
            faux.unregister();
            let _ = fs::remove_dir_all(cleanup_cwd);
        },
    )
}

#[test]
fn detects_missing_session_cwd_from_persisted_sessions() {
    let fallback_cwd = unique_temp_dir("pi-session-cwd-fallback");
    let missing_cwd = fallback_cwd.join("does-not-exist");
    let session_dir = unique_temp_dir("pi-session-cwd-session-dir");
    let session_file = session_dir.join("session.jsonl");
    write_session_file(&session_file, &missing_cwd);

    let session_manager = SessionManager::open(session_file.to_str().unwrap(), None, None).unwrap();
    let issue = get_missing_session_cwd_issue(&session_manager, &fallback_cwd)
        .expect("expected missing session cwd issue");

    assert_eq!(
        issue,
        SessionCwdIssue {
            session_file: Some(session_file.to_string_lossy().into_owned()),
            session_cwd: missing_cwd.to_string_lossy().into_owned(),
            fallback_cwd: fallback_cwd.to_string_lossy().into_owned(),
        }
    );
    assert_eq!(
        assert_session_cwd_exists(&session_manager, &fallback_cwd).unwrap_err(),
        issue
    );

    let _ = fs::remove_dir_all(&fallback_cwd);
    let _ = fs::remove_dir_all(&session_dir);
}

#[test]
fn supports_overriding_the_effective_cwd_when_opening_a_session() {
    let fallback_cwd = unique_temp_dir("pi-session-cwd-override");
    let missing_cwd = fallback_cwd.join("does-not-exist");
    let session_dir = unique_temp_dir("pi-session-cwd-override-session-dir");
    let session_file = session_dir.join("session.jsonl");
    write_session_file(&session_file, &missing_cwd);

    let session_manager = SessionManager::open(
        session_file.to_str().unwrap(),
        None,
        Some(fallback_cwd.to_str().unwrap()),
    )
    .unwrap();

    assert_eq!(session_manager.get_cwd(), fallback_cwd.to_string_lossy().as_ref());
    assert!(get_missing_session_cwd_issue(&session_manager, &fallback_cwd).is_none());
    assert!(assert_session_cwd_exists(&session_manager, &fallback_cwd).is_ok());

    let _ = fs::remove_dir_all(&fallback_cwd);
    let _ = fs::remove_dir_all(&session_dir);
}

#[tokio::test]
async fn create_agent_session_runtime_errors_before_factory_when_stored_cwd_is_missing() {
    let fallback_cwd = unique_temp_dir("pi-session-cwd-runtime");
    let missing_cwd = fallback_cwd.join("does-not-exist");
    let session_dir = unique_temp_dir("pi-session-cwd-runtime-session-dir");
    let session_file = session_dir.join("session.jsonl");
    write_session_file(&session_file, &missing_cwd);

    let session_manager = SessionManager::open(session_file.to_str().unwrap(), None, None).unwrap();
    let create_runtime_called = Arc::new(AtomicBool::new(false));
    let create_runtime_called_for_factory = create_runtime_called.clone();
    let factory: CreateAgentSessionRuntimeFactory = Arc::new(move |_request| {
        create_runtime_called_for_factory.store(true, Ordering::Relaxed);
        Box::pin(async { panic!("create runtime should not be called") })
    });

    let error = match create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: fallback_cwd.clone(),
            session_manager: Some(Arc::new(Mutex::new(session_manager))),
        },
    )
    .await
    {
        Ok(_) => panic!("expected missing session cwd error"),
        Err(error) => error,
    };

    match error {
        AgentSessionRuntimeError::MissingSessionCwd(issue) => {
            assert_eq!(issue.session_file, Some(session_file.to_string_lossy().into_owned()));
            assert_eq!(issue.session_cwd, missing_cwd.to_string_lossy().into_owned());
            assert_eq!(issue.fallback_cwd, fallback_cwd.to_string_lossy().into_owned());
        }
        other => panic!("expected missing session cwd error, got {other:?}"),
    }
    assert!(!create_runtime_called.load(Ordering::Relaxed));

    let _ = fs::remove_dir_all(&fallback_cwd);
    let _ = fs::remove_dir_all(&session_dir);
}

#[tokio::test]
async fn switch_session_reports_missing_session_cwd_with_current_runtime_as_fallback() {
    let fallback_cwd = unique_temp_dir("pi-session-cwd-switch");
    let missing_cwd = fallback_cwd.join("does-not-exist");
    let session_dir = unique_temp_dir("pi-session-cwd-switch-session-dir");
    let session_file = session_dir.join("session.jsonl");
    write_session_file(&session_file, &missing_cwd);

    let (factory, cleanup) = build_runtime_factory(&fallback_cwd);
    let initial_manager = SessionManager::in_memory(fallback_cwd.to_string_lossy().as_ref());
    let mut runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: fallback_cwd.clone(),
            session_manager: Some(Arc::new(Mutex::new(initial_manager))),
        },
    )
    .await
    .expect("expected initial runtime");

    let error = runtime
        .switch_session(session_file.to_str().unwrap(), None)
        .await
        .expect_err("expected missing session cwd error");

    match error {
        AgentSessionRuntimeError::MissingSessionCwd(issue) => {
            assert_eq!(issue.session_file, Some(session_file.to_string_lossy().into_owned()));
            assert_eq!(issue.session_cwd, missing_cwd.to_string_lossy().into_owned());
            assert_eq!(issue.fallback_cwd, fallback_cwd.to_string_lossy().into_owned());
        }
        other => panic!("expected missing session cwd error, got {other:?}"),
    }

    cleanup();
    let _ = fs::remove_dir_all(&session_dir);
}
