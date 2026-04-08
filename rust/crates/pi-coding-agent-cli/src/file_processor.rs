use base64::{Engine as _, engine::general_purpose::STANDARD};
use pi_coding_agent_tools::{
    detect_supported_image_mime_type, format_dimension_note, resize_image_bytes, resolve_read_path,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessFileOptions {
    pub auto_resize_images: bool,
}

impl Default for ProcessFileOptions {
    fn default() -> Self {
        Self {
            auto_resize_images: true,
        }
    }
}

pub fn process_file_arguments(
    file_args: &[String],
    cwd: &Path,
    options: ProcessFileOptions,
) -> Result<ProcessedFiles, String> {
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
            if options.auto_resize_images {
                let Some(resized) = resize_image_bytes(&bytes, mime_type, None) else {
                    text.push_str(&format!(
                        "<file name=\"{}\">[Image omitted: could not be resized below the inline image size limit.]</file>\n",
                        absolute_path.display()
                    ));
                    continue;
                };

                let dimension_note = format_dimension_note(&resized);
                images.push(UserContent::Image {
                    data: resized.data,
                    mime_type: resized.mime_type,
                });
                if let Some(dimension_note) = dimension_note {
                    text.push_str(&format!(
                        "<file name=\"{}\">{dimension_note}</file>\n",
                        absolute_path.display()
                    ));
                } else {
                    text.push_str(&format!(
                        "<file name=\"{}\"></file>\n",
                        absolute_path.display()
                    ));
                }
            } else {
                images.push(UserContent::Image {
                    data: STANDARD.encode(bytes),
                    mime_type: mime_type.to_string(),
                });
                text.push_str(&format!(
                    "<file name=\"{}\"></file>\n",
                    absolute_path.display()
                ));
            }
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
