use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSkillBlock {
    pub name: String,
    pub location: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
}

pub fn parse_skill_block(text: &str) -> Option<ParsedSkillBlock> {
    let text = text.strip_prefix("<skill name=\"")?;
    let (name, text) = text.split_once("\" location=\"")?;
    let (location, text) = text.split_once("\">\n")?;
    let (content, suffix) = text.split_once("\n</skill>")?;

    let user_message = if suffix.is_empty() {
        None
    } else {
        let suffix = suffix.strip_prefix("\n\n")?;
        let trimmed = suffix.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };

    Some(ParsedSkillBlock {
        name: name.to_string(),
        location: location.to_string(),
        content: content.to_string(),
        user_message,
    })
}
