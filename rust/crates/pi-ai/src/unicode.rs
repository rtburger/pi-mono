/// Compatibility text sanitization shared by Rust provider request builders.
///
/// The TypeScript implementation removes unpaired UTF-16 surrogate code units before
/// JSON serialization (`packages/ai/src/utils/sanitize-unicode.ts`). Rust `String`
/// values are always valid UTF-8 and therefore cannot contain standalone surrogate
/// code units, so the problematic TypeScript-only state is unrepresentable after text
/// crosses into the Rust migration surface.
///
/// Keep request builders calling this helper anyway so the compatibility boundary stays
/// explicit and future input-path changes have a single hook.
pub(crate) fn sanitize_provider_text(text: &str) -> String {
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::sanitize_provider_text;

    #[test]
    fn preserves_valid_non_bmp_and_multilingual_text() {
        let text =
            "Mario Zechner wann? Wo? Bin grad äußersr eventuninformiert 🙈 こんにちは 你好 ∑∫∂√";
        assert_eq!(sanitize_provider_text(text), text);
    }

    #[test]
    fn preserves_lossy_utf16_fallback_text_after_rust_string_boundary() {
        let lossy = String::from_utf16_lossy(&[0xD83D]);
        assert_eq!(lossy, "\u{FFFD}");
        assert_eq!(sanitize_provider_text(&lossy), lossy);
    }
}
