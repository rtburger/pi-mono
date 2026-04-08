use pi_events::UserContent;

#[derive(Debug, Clone, PartialEq)]
pub struct InitialMessageResult {
    pub initial_message: Option<String>,
    pub initial_images: Option<Vec<UserContent>>,
}

pub fn build_initial_message(
    messages: &mut Vec<String>,
    file_text: Option<String>,
    file_images: Vec<UserContent>,
    stdin_content: Option<String>,
) -> InitialMessageResult {
    let mut parts = Vec::new();

    if let Some(stdin_content) = stdin_content {
        parts.push(stdin_content);
    }
    if let Some(file_text) = file_text
        && !file_text.is_empty()
    {
        parts.push(file_text);
    }
    if let Some(first_message) = messages.first().cloned() {
        parts.push(first_message);
        messages.remove(0);
    }

    InitialMessageResult {
        initial_message: (!parts.is_empty()).then(|| parts.join("")),
        initial_images: (!file_images.is_empty()).then_some(file_images),
    }
}
