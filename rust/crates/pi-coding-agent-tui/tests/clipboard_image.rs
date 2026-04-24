use parking_lot::Mutex;
use pi_coding_agent_tui::{
    ClipboardCommandRunner, ClipboardImage, ClipboardImageSource, ClipboardPlatform, CommandOutput,
    KeybindingsManager, PlainKeyHintStyler, StartupShellComponent, SystemClipboardImageSource,
    extension_for_image_mime_type, is_wayland_session, paste_clipboard_image_into_shell,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::tempdir;

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&path).expect("failed to create temp dir");
    path
}

fn create_tiny_bmp_1x1_red() -> Vec<u8> {
    let mut buffer = vec![0; 58];
    buffer[0..2].copy_from_slice(b"BM");
    buffer[2..6].copy_from_slice(&(58u32).to_le_bytes());
    buffer[10..14].copy_from_slice(&(54u32).to_le_bytes());
    buffer[14..18].copy_from_slice(&(40u32).to_le_bytes());
    buffer[18..22].copy_from_slice(&(1i32).to_le_bytes());
    buffer[22..26].copy_from_slice(&(1i32).to_le_bytes());
    buffer[26..28].copy_from_slice(&(1u16).to_le_bytes());
    buffer[28..30].copy_from_slice(&(24u16).to_le_bytes());
    buffer[34..38].copy_from_slice(&(4u32).to_le_bytes());
    buffer[54] = 0x00;
    buffer[55] = 0x00;
    buffer[56] = 0xff;
    buffer[57] = 0x00;
    buffer
}

#[derive(Default)]
struct FakeRunner {
    calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    responses: BTreeMap<(String, String), CommandOutput>,
}

impl FakeRunner {
    fn with_responses(responses: Vec<((String, Vec<String>), CommandOutput)>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            responses: responses
                .into_iter()
                .map(|((command, args), output)| ((command, args.join("\u{0}")), output))
                .collect(),
        }
    }
}

impl ClipboardCommandRunner for FakeRunner {
    fn run(
        &self,
        command: &str,
        args: &[String],
        _env: &BTreeMap<String, String>,
    ) -> CommandOutput {
        self.calls.lock().push((command.to_owned(), args.to_vec()));
        self.responses
            .get(&(command.to_owned(), args.join("\u{0}")))
            .cloned()
            .unwrap_or(CommandOutput {
                ok: false,
                stdout: Vec::new(),
            })
    }
}

struct StaticClipboardImageSource {
    image: Option<ClipboardImage>,
}

impl ClipboardImageSource for StaticClipboardImageSource {
    fn read_clipboard_image(&self) -> std::io::Result<Option<ClipboardImage>> {
        Ok(self.image.clone())
    }
}

fn shell() -> StartupShellComponent {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    StartupShellComponent::new("Pi", "1.2.3", &keybindings, &PlainKeyHintStyler, true)
}

#[test]
fn extension_for_image_mime_type_maps_supported_types() {
    assert_eq!(extension_for_image_mime_type("image/png"), Some("png"));
    assert_eq!(extension_for_image_mime_type("image/jpeg"), Some("jpg"));
    assert_eq!(extension_for_image_mime_type("image/webp"), Some("webp"));
    assert_eq!(extension_for_image_mime_type("image/gif"), Some("gif"));
    assert_eq!(
        extension_for_image_mime_type("image/png; charset=binary"),
        Some("png")
    );
    assert_eq!(extension_for_image_mime_type("image/bmp"), None);
}

#[test]
fn wayland_detection_matches_typescript_shape() {
    let mut env = BTreeMap::new();
    assert!(!is_wayland_session(&env));

    env.insert("WAYLAND_DISPLAY".to_owned(), "1".to_owned());
    assert!(is_wayland_session(&env));

    env.clear();
    env.insert("XDG_SESSION_TYPE".to_owned(), "wayland".to_owned());
    assert!(is_wayland_session(&env));
}

#[test]
fn system_clipboard_source_prefers_wl_paste_on_wayland() {
    let runner = FakeRunner::with_responses(vec![
        (
            (("wl-paste".to_owned(), vec!["--list-types".to_owned()])),
            CommandOutput {
                ok: true,
                stdout: b"text/plain\nimage/png\n".to_vec(),
            },
        ),
        (
            (
                "wl-paste".to_owned(),
                vec![
                    "--type".to_owned(),
                    "image/png".to_owned(),
                    "--no-newline".to_owned(),
                ],
            ),
            CommandOutput {
                ok: true,
                stdout: vec![1, 2, 3],
            },
        ),
    ]);
    let mut env = BTreeMap::new();
    env.insert("WAYLAND_DISPLAY".to_owned(), "1".to_owned());

    let source = SystemClipboardImageSource::with_parts(
        env,
        ClipboardPlatform::Linux,
        unique_temp_dir("clipboard-wl-paste"),
        Box::new(runner),
    );

    let image = source
        .read_clipboard_image()
        .expect("clipboard read should succeed")
        .expect("expected clipboard image");
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(image.bytes, vec![1, 2, 3]);
}

#[test]
fn system_clipboard_source_falls_back_to_xclip_when_wl_paste_is_unavailable() {
    let runner = FakeRunner::with_responses(vec![
        (
            (
                "xclip".to_owned(),
                vec![
                    "-selection".to_owned(),
                    "clipboard".to_owned(),
                    "-t".to_owned(),
                    "TARGETS".to_owned(),
                    "-o".to_owned(),
                ],
            ),
            CommandOutput {
                ok: true,
                stdout: b"image/png\n".to_vec(),
            },
        ),
        (
            (
                "xclip".to_owned(),
                vec![
                    "-selection".to_owned(),
                    "clipboard".to_owned(),
                    "-t".to_owned(),
                    "image/png".to_owned(),
                    "-o".to_owned(),
                ],
            ),
            CommandOutput {
                ok: true,
                stdout: vec![9, 8],
            },
        ),
    ]);
    let mut env = BTreeMap::new();
    env.insert("XDG_SESSION_TYPE".to_owned(), "wayland".to_owned());

    let source = SystemClipboardImageSource::with_parts(
        env,
        ClipboardPlatform::Linux,
        unique_temp_dir("clipboard-xclip"),
        Box::new(runner),
    );

    let image = source
        .read_clipboard_image()
        .expect("clipboard read should succeed")
        .expect("expected clipboard image");
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(image.bytes, vec![9, 8]);
}

#[test]
fn system_clipboard_source_converts_unsupported_bmp_to_png() {
    let runner = FakeRunner::with_responses(vec![
        (
            (("wl-paste".to_owned(), vec!["--list-types".to_owned()])),
            CommandOutput {
                ok: true,
                stdout: b"image/bmp\n".to_vec(),
            },
        ),
        (
            (
                "wl-paste".to_owned(),
                vec![
                    "--type".to_owned(),
                    "image/bmp".to_owned(),
                    "--no-newline".to_owned(),
                ],
            ),
            CommandOutput {
                ok: true,
                stdout: create_tiny_bmp_1x1_red(),
            },
        ),
    ]);
    let mut env = BTreeMap::new();
    env.insert("WAYLAND_DISPLAY".to_owned(), "1".to_owned());

    let source = SystemClipboardImageSource::with_parts(
        env,
        ClipboardPlatform::Linux,
        unique_temp_dir("clipboard-bmp"),
        Box::new(runner),
    );

    let image = source
        .read_clipboard_image()
        .expect("clipboard read should succeed")
        .expect("expected clipboard image");
    assert_eq!(image.mime_type, "image/png");
    assert!(image.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn system_clipboard_source_returns_none_in_termux() {
    let runner = FakeRunner::default();
    let mut env = BTreeMap::new();
    env.insert("TERMUX_VERSION".to_owned(), "1.0".to_owned());

    let source = SystemClipboardImageSource::with_parts(
        env,
        ClipboardPlatform::Linux,
        unique_temp_dir("clipboard-termux"),
        Box::new(runner),
    );

    let image = source
        .read_clipboard_image()
        .expect("clipboard read should succeed");
    assert!(image.is_none());
}

#[test]
fn paste_clipboard_image_into_shell_writes_temp_file_and_inserts_path_at_cursor() {
    let temp_dir = tempdir().expect("tempdir should be created");
    let mut shell = shell();
    shell.set_input_value("prefix  suffix");
    shell.set_input_cursor("prefix ".len());

    let source = StaticClipboardImageSource {
        image: Some(ClipboardImage {
            bytes: vec![1, 2, 3],
            mime_type: "image/png".to_owned(),
        }),
    };

    let written_path = paste_clipboard_image_into_shell(&mut shell, &source, temp_dir.path())
        .expect("clipboard paste should succeed")
        .expect("expected written file path");

    assert!(written_path.exists());
    assert_eq!(
        fs::read(&written_path).expect("file should be readable"),
        vec![1, 2, 3]
    );
    assert!(written_path.extension().is_some_and(|ext| ext == "png"));

    let expected = format!("prefix {} suffix", written_path.to_string_lossy());
    assert_eq!(shell.input_value(), expected);
}

#[test]
fn paste_clipboard_image_into_shell_returns_none_when_clipboard_is_empty() {
    let temp_dir = tempdir().expect("tempdir should be created");
    let mut shell = shell();
    shell.set_input_value("draft");

    let source = StaticClipboardImageSource { image: None };

    let result = paste_clipboard_image_into_shell(&mut shell, &source, temp_dir.path())
        .expect("clipboard paste should succeed");

    assert!(result.is_none());
    assert_eq!(shell.input_value(), "draft");
    assert!(
        temp_dir
            .path()
            .read_dir()
            .expect("temp dir readable")
            .next()
            .is_none()
    );
}
