use serde::de::DeserializeOwned;

pub fn parse_frontmatter<T>(raw: &str) -> (Option<T>, &str)
where
    T: DeserializeOwned,
{
    let Some(body) = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
    else {
        return (None, raw);
    };

    let mut yaml = String::new();
    let mut consumed = raw.len() - body.len();
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        consumed += line.len();
        if trimmed == "---" || trimmed == "..." {
            let parsed = serde_yaml::from_str::<T>(&yaml).ok();
            return (parsed, &raw[consumed..]);
        }
        yaml.push_str(line);
    }

    (None, raw)
}

pub fn strip_frontmatter(raw: &str) -> &str {
    parse_frontmatter::<serde_yaml::Value>(raw).1
}
