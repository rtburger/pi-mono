use crate::StartupShellComponent;
use image::ImageFormat;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Cursor},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SUPPORTED_IMAGE_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

type ClipboardEnv = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

pub trait ClipboardImageSource {
    fn read_clipboard_image(&self) -> io::Result<Option<ClipboardImage>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardPlatform {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl ClipboardPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub ok: bool,
    pub stdout: Vec<u8>,
}

pub trait ClipboardCommandRunner: Send + Sync {
    fn run(&self, command: &str, args: &[String], env: &ClipboardEnv) -> CommandOutput;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StdClipboardCommandRunner;

impl ClipboardCommandRunner for StdClipboardCommandRunner {
    fn run(&self, command: &str, args: &[String], env: &ClipboardEnv) -> CommandOutput {
        let mut process = Command::new(command);
        process.args(args);
        if !env.is_empty() {
            process.envs(env);
        }

        match process.output() {
            Ok(output) => CommandOutput {
                ok: output.status.success(),
                stdout: output.stdout,
            },
            Err(_) => CommandOutput {
                ok: false,
                stdout: Vec::new(),
            },
        }
    }
}

pub struct SystemClipboardImageSource {
    env: ClipboardEnv,
    platform: ClipboardPlatform,
    temp_dir: PathBuf,
    runner: Box<dyn ClipboardCommandRunner>,
}

impl Default for SystemClipboardImageSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClipboardImageSource {
    pub fn new() -> Self {
        Self {
            env: std::env::vars().collect(),
            platform: ClipboardPlatform::current(),
            temp_dir: std::env::temp_dir(),
            runner: Box::new(StdClipboardCommandRunner),
        }
    }

    pub fn with_parts(
        env: ClipboardEnv,
        platform: ClipboardPlatform,
        temp_dir: PathBuf,
        runner: Box<dyn ClipboardCommandRunner>,
    ) -> Self {
        Self {
            env,
            platform,
            temp_dir,
            runner,
        }
    }
}

impl ClipboardImageSource for SystemClipboardImageSource {
    fn read_clipboard_image(&self) -> io::Result<Option<ClipboardImage>> {
        read_clipboard_image_from_system(&*self.runner, &self.env, self.platform, &self.temp_dir)
    }
}

pub fn is_wayland_session(env: &ClipboardEnv) -> bool {
    env.contains_key("WAYLAND_DISPLAY")
        || env
            .get("XDG_SESSION_TYPE")
            .is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
}

pub fn extension_for_image_mime_type(mime_type: &str) -> Option<&'static str> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

pub fn paste_clipboard_image_into_shell<S: ClipboardImageSource>(
    shell: &mut StartupShellComponent,
    source: &S,
    temp_dir: &Path,
) -> io::Result<Option<PathBuf>> {
    let Some(image) = source.read_clipboard_image()? else {
        return Ok(None);
    };

    fs::create_dir_all(temp_dir)?;

    let extension = extension_for_image_mime_type(&image.mime_type).unwrap_or("png");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_path = temp_dir.join(format!(
        "pi-clipboard-{}-{unique}.{extension}",
        std::process::id()
    ));

    fs::write(&file_path, &image.bytes)?;
    shell.insert_input_text_at_cursor(&file_path.to_string_lossy());
    Ok(Some(file_path))
}

fn read_clipboard_image_from_system(
    runner: &dyn ClipboardCommandRunner,
    env: &ClipboardEnv,
    platform: ClipboardPlatform,
    temp_dir: &Path,
) -> io::Result<Option<ClipboardImage>> {
    if env.contains_key("TERMUX_VERSION") {
        return Ok(None);
    }

    if platform != ClipboardPlatform::Linux {
        return Ok(None);
    }

    let wayland = is_wayland_session(env);
    let wsl = is_wsl(env);

    if (wayland || wsl)
        && let Some(image) = read_clipboard_image_via_wl_paste(runner, env)?
    {
        return Ok(Some(image));
    }

    if let Some(image) = read_clipboard_image_via_xclip(runner, env)? {
        return Ok(Some(image));
    }

    if wsl && let Some(image) = read_clipboard_image_via_powershell(runner, env, temp_dir)? {
        return Ok(Some(image));
    }

    Ok(None)
}

fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_lowercase()
}

fn is_supported_image_mime_type(mime_type: &str) -> bool {
    let base = base_mime_type(mime_type);
    SUPPORTED_IMAGE_MIME_TYPES
        .iter()
        .any(|candidate| *candidate == base)
}

fn select_preferred_image_mime_type(mime_types: &[String]) -> Option<String> {
    let normalized = mime_types
        .iter()
        .map(|mime_type| mime_type.trim())
        .filter(|mime_type| !mime_type.is_empty())
        .map(|mime_type| (mime_type.to_owned(), base_mime_type(mime_type)))
        .collect::<Vec<_>>();

    for preferred in SUPPORTED_IMAGE_MIME_TYPES {
        if let Some((raw, _)) = normalized.iter().find(|(_, base)| base == preferred) {
            return Some(raw.clone());
        }
    }

    normalized
        .into_iter()
        .find(|(_, base)| base.starts_with("image/"))
        .map(|(raw, _)| raw)
}

fn normalize_image(image: ClipboardImage) -> io::Result<Option<ClipboardImage>> {
    let mime_type = base_mime_type(&image.mime_type);
    if is_supported_image_mime_type(&mime_type) {
        return Ok(Some(ClipboardImage {
            bytes: image.bytes,
            mime_type,
        }));
    }

    if !mime_type.starts_with("image/") {
        return Ok(None);
    }

    let dynamic = match image::load_from_memory(&image.bytes) {
        Ok(dynamic) => dynamic,
        Err(_) => return Ok(None),
    };
    let mut cursor = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(io::Error::other)?;
    Ok(Some(ClipboardImage {
        bytes: cursor.into_inner(),
        mime_type: "image/png".to_owned(),
    }))
}

fn read_clipboard_image_via_wl_paste(
    runner: &dyn ClipboardCommandRunner,
    env: &ClipboardEnv,
) -> io::Result<Option<ClipboardImage>> {
    let list = runner.run("wl-paste", &["--list-types".to_owned()], env);
    if !list.ok {
        return Ok(None);
    }

    let mime_types = String::from_utf8_lossy(&list.stdout)
        .split('\n')
        .map(str::trim)
        .filter(|mime_type| !mime_type.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let Some(selected_type) = select_preferred_image_mime_type(&mime_types) else {
        return Ok(None);
    };

    let data = runner.run(
        "wl-paste",
        &[
            "--type".to_owned(),
            selected_type.clone(),
            "--no-newline".to_owned(),
        ],
        env,
    );
    if !data.ok || data.stdout.is_empty() {
        return Ok(None);
    }

    normalize_image(ClipboardImage {
        bytes: data.stdout,
        mime_type: selected_type,
    })
}

fn read_clipboard_image_via_xclip(
    runner: &dyn ClipboardCommandRunner,
    env: &ClipboardEnv,
) -> io::Result<Option<ClipboardImage>> {
    let targets = runner.run(
        "xclip",
        &[
            "-selection".to_owned(),
            "clipboard".to_owned(),
            "-t".to_owned(),
            "TARGETS".to_owned(),
            "-o".to_owned(),
        ],
        env,
    );

    let candidate_types = if targets.ok {
        String::from_utf8_lossy(&targets.stdout)
            .split('\n')
            .map(str::trim)
            .filter(|mime_type| !mime_type.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let preferred = select_preferred_image_mime_type(&candidate_types);
    let mut try_types = Vec::new();
    if let Some(preferred) = preferred {
        try_types.push(preferred);
    }
    for supported in SUPPORTED_IMAGE_MIME_TYPES {
        if !try_types.iter().any(|candidate| candidate == supported) {
            try_types.push((*supported).to_owned());
        }
    }

    for mime_type in try_types {
        let data = runner.run(
            "xclip",
            &[
                "-selection".to_owned(),
                "clipboard".to_owned(),
                "-t".to_owned(),
                mime_type.clone(),
                "-o".to_owned(),
            ],
            env,
        );
        if data.ok && !data.stdout.is_empty() {
            return normalize_image(ClipboardImage {
                bytes: data.stdout,
                mime_type,
            });
        }
    }

    Ok(None)
}

fn read_clipboard_image_via_powershell(
    runner: &dyn ClipboardCommandRunner,
    env: &ClipboardEnv,
    temp_dir: &Path,
) -> io::Result<Option<ClipboardImage>> {
    fs::create_dir_all(temp_dir)?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_file = temp_dir.join(format!(
        "pi-wsl-clipboard-{}-{unique}.png",
        std::process::id()
    ));

    let win_path = runner.run(
        "wslpath",
        &["-w".to_owned(), temp_file.display().to_string()],
        env,
    );
    if !win_path.ok {
        return Ok(None);
    }

    let win_path = String::from_utf8_lossy(&win_path.stdout).trim().to_owned();
    if win_path.is_empty() {
        return Ok(None);
    }

    let script = [
        "Add-Type -AssemblyName System.Windows.Forms",
        "Add-Type -AssemblyName System.Drawing",
        "$path = $env:PI_WSL_CLIPBOARD_IMAGE_PATH",
        "$img = [System.Windows.Forms.Clipboard]::GetImage()",
        "if ($img) { $img.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok' } else { Write-Output 'empty' }",
    ]
    .join("; ");

    let mut powershell_env = env.clone();
    powershell_env.insert("PI_WSL_CLIPBOARD_IMAGE_PATH".to_owned(), win_path);

    let result = runner.run(
        "powershell.exe",
        &["-NoProfile".to_owned(), "-Command".to_owned(), script],
        &powershell_env,
    );
    if !result.ok || String::from_utf8_lossy(&result.stdout).trim() != "ok" {
        let _ = fs::remove_file(&temp_file);
        return Ok(None);
    }

    let bytes = fs::read(&temp_file).ok();
    let _ = fs::remove_file(&temp_file);
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }

    Ok(Some(ClipboardImage {
        bytes,
        mime_type: "image/png".to_owned(),
    }))
}

fn is_wsl(env: &ClipboardEnv) -> bool {
    if env.contains_key("WSL_DISTRO_NAME") || env.contains_key("WSLENV") {
        return true;
    }

    fs::read_to_string("/proc/version")
        .map(|release| {
            let release = release.to_ascii_lowercase();
            release.contains("microsoft") || release.contains("wsl")
        })
        .unwrap_or(false)
}
