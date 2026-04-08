use base64::{Engine as _, engine::general_purpose::STANDARD};
use pi_coding_agent_tools::{detect_supported_image_mime_type, resolve_read_path};
use pi_events::UserContent;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedFiles {
    pub text: String,
    pub images: Vec<UserContent>,
}

pub fn process_file_arguments(file_args: &[String], cwd: &Path) -> Result<ProcessedFiles, String> {
    let mut text = String::new();
    let mut images = Vec::new();

    for file_arg in file_args {
        let absolute_path = resolve_file_argument_path(file_arg, cwd);

        if !absolute_path.exists() {
            return Err(format!("File not found: {}", absolute_path.display()));
        }

        let metadata = fs::metadata(&absolute_path)
            .map_err(|error| format!("Could not stat file {}: {error}", absolute_path.display()))?;
        if metadata.len() == 0 {
            continue;
        }

        let bytes = fs::read(&absolute_path)
            .map_err(|error| format!("Could not read file {}: {error}", absolute_path.display()))?;

        if let Some(mime_type) = detect_supported_image_mime_type(&bytes) {
            images.push(UserContent::Image {
                data: STANDARD.encode(bytes),
                mime_type: mime_type.to_string(),
            });
            text.push_str(&format!(
                "<file name=\"{}\"></file>\n",
                absolute_path.display()
            ));
            continue;
        }

        let file_text = String::from_utf8(bytes)
            .map_err(|error| format!("Could not read file {}: {error}", absolute_path.display()))?;
        text.push_str(&format!(
            "<file name=\"{}\">\n{}\n</file>\n",
            absolute_path.display(),
            file_text
        ));
    }

    Ok(ProcessedFiles { text, images })
}

fn resolve_file_argument_path(file_arg: &str, cwd: &Path) -> PathBuf {
    let resolved = resolve_read_path(file_arg, cwd);
    if resolved.is_absolute() {
        resolved
    } else {
        cwd.join(resolved)
    }
}
