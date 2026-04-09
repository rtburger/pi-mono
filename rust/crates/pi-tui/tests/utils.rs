use pi_tui::{
    extract_segments, slice_by_column, slice_with_width, truncate_to_width, visible_width,
    wrap_text_with_ansi,
};

#[test]
fn truncate_keeps_output_within_width_for_large_unicode_input() {
    let text = "🙂界".repeat(100_000);
    let truncated = truncate_to_width(&text, 40, "…", false);

    assert!(visible_width(&truncated) <= 40);
    assert!(truncated.ends_with("…\x1b[0m"));
}

#[test]
fn truncate_preserves_ansi_styling_and_resets_around_ellipsis() {
    let text = format!("\x1b[31m{}\x1b[0m", "hello ".repeat(1000));
    let truncated = truncate_to_width(&text, 20, "…", false);

    assert!(visible_width(&truncated) <= 20);
    assert!(truncated.contains("\x1b[31m"));
    assert!(truncated.ends_with("\x1b[0m…\x1b[0m"));
}

#[test]
fn truncate_handles_malformed_ansi_prefixes_without_hanging() {
    let text = format!("abc\x1bnot-ansi {}", "🙂".repeat(1000));
    let truncated = truncate_to_width(&text, 20, "…", false);

    assert!(visible_width(&truncated) <= 20);
}

#[test]
fn truncate_clips_wide_ellipsis_safely() {
    assert_eq!(truncate_to_width("abcdef", 1, "🙂", false), "");
    assert_eq!(
        truncate_to_width("abcdef", 2, "🙂", false),
        "\x1b[0m🙂\x1b[0m"
    );
    assert!(visible_width(&truncate_to_width("abcdef", 2, "🙂", false)) <= 2);
}

#[test]
fn truncate_returns_original_text_when_it_already_fits_even_if_ellipsis_is_too_wide() {
    assert_eq!(truncate_to_width("a", 2, "🙂", false), "a");
    assert_eq!(truncate_to_width("界", 2, "🙂", false), "界");
}

#[test]
fn truncate_pads_to_requested_width() {
    let truncated = truncate_to_width("🙂界🙂界🙂界", 8, "…", true);
    assert_eq!(visible_width(&truncated), 8);
}

#[test]
fn truncate_adds_trailing_reset_when_truncating_without_an_ellipsis() {
    let truncated = truncate_to_width(&format!("\x1b[31m{}", "hello".repeat(100)), 10, "", false);

    assert!(visible_width(&truncated) <= 10);
    assert!(truncated.ends_with("\x1b[0m"));
}

#[test]
fn truncate_keeps_a_contiguous_prefix() {
    let truncated = truncate_to_width("🙂\t界 \x1b_abc\x07", 7, "…", true);
    assert_eq!(truncated, "🙂\t\x1b[0m…\x1b[0m ");
}

#[test]
fn visible_width_counts_tabs_inline_and_skips_ansi_inline() {
    assert_eq!(visible_width("\t\x1b[31m界\x1b[0m"), 5);
}

#[test]
fn wrap_plain_text_to_width() {
    let wrapped = wrap_text_with_ansi("hello world this is a test", 10);

    assert!(wrapped.len() > 1);
    for line in wrapped {
        assert!(visible_width(&line) <= 10);
    }
}

#[test]
fn visible_width_ignores_osc_sequences() {
    assert_eq!(visible_width("\x1b]133;A\x07hello\x1b]133;B\x07"), 5);
    assert_eq!(visible_width("\x1b]133;A\x1b\\hello\x1b]133;B\x1b\\"), 5);
}

#[test]
fn visible_width_treats_isolated_and_paired_regional_indicators_as_full_width() {
    assert_eq!(visible_width("🇨"), 2);
    assert_eq!(visible_width("🇨🇳"), 2);
    assert_eq!(visible_width("      - 🇨"), 10);
}

#[test]
fn wrap_intermediate_partial_flag_before_overflow() {
    let wrapped = wrap_text_with_ansi("      - 🇨", 9);

    assert_eq!(wrapped.len(), 2);
    assert_eq!(visible_width(&wrapped[0]), 7);
    assert_eq!(visible_width(&wrapped[1]), 2);
}

#[test]
fn visible_width_keeps_common_streaming_emoji_intermediates_stable() {
    for sample in ["👍", "👍🏻", "✅", "⚡", "⚡️", "👨", "👨‍💻", "🏳️‍🌈"]
    {
        assert_eq!(visible_width(sample), 2, "sample: {sample}");
    }
}

#[test]
fn wrap_does_not_apply_underline_before_the_styled_text() {
    let underline_on = "\x1b[4m";
    let underline_off = "\x1b[24m";
    let url = "https://example.com/very/long/path/that/will/wrap";
    let text = format!("read this thread {underline_on}{url}{underline_off}");

    let wrapped = wrap_text_with_ansi(&text, 40);

    assert_eq!(wrapped[0], "read this thread");
    assert!(wrapped[1].starts_with(underline_on));
    assert!(wrapped[1].contains("https://"));
}

#[test]
fn wrap_does_not_leave_whitespace_before_underline_reset() {
    let underline_on = "\x1b[4m";
    let underline_off = "\x1b[24m";
    let text = format!("{underline_on}underlined text here {underline_off}more");

    let wrapped = wrap_text_with_ansi(&text, 18);
    assert!(!wrapped[0].contains(&format!(" {underline_off}")));
}

#[test]
fn wrap_uses_underline_only_reset_at_line_end() {
    let underline_on = "\x1b[4m";
    let underline_off = "\x1b[24m";
    let url = "https://example.com/very/long/path/that/will/definitely/wrap";
    let text = format!("prefix {underline_on}{url}{underline_off} suffix");

    let wrapped = wrap_text_with_ansi(&text, 30);
    for line in wrapped.iter().skip(1).take(wrapped.len().saturating_sub(2)) {
        if line.contains(underline_on) {
            assert!(line.ends_with(underline_off));
            assert!(!line.ends_with("\x1b[0m"));
        }
    }
}

#[test]
fn wrap_preserves_background_across_wrapped_lines() {
    let bg_blue = "\x1b[44m";
    let reset = "\x1b[0m";
    let text = format!("{bg_blue}hello world this is blue background text{reset}");

    let wrapped = wrap_text_with_ansi(&text, 15);
    for line in &wrapped {
        assert!(line.contains(bg_blue));
    }
    for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
        assert!(!line.ends_with("\x1b[0m"));
    }
}

#[test]
fn wrap_resets_underline_but_preserves_background() {
    let underline_on = "\x1b[4m";
    let underline_off = "\x1b[24m";
    let reset = "\x1b[0m";
    let text = format!(
        "\x1b[41mprefix {underline_on}UNDERLINED_CONTENT_THAT_WRAPS{underline_off} suffix{reset}"
    );

    let wrapped = wrap_text_with_ansi(&text, 20);

    for line in &wrapped {
        let has_bg_color = line.contains("[41m") || line.contains(";41m") || line.contains("[41;");
        assert!(has_bg_color);
    }

    for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
        let has_underline = (line.contains("[4m") || line.contains("[4;") || line.contains(";4m"))
            && !line.contains(underline_off);
        if has_underline {
            assert!(line.ends_with(underline_off));
            assert!(!line.ends_with("\x1b[0m"));
        }
    }
}

#[test]
fn wrap_preserves_color_codes_across_wraps() {
    let red = "\x1b[31m";
    let reset = "\x1b[0m";
    let text = format!("{red}hello world this is red{reset}");

    let wrapped = wrap_text_with_ansi(&text, 10);
    for line in wrapped.iter().skip(1) {
        assert!(line.starts_with(red));
    }
    for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
        assert!(!line.ends_with("\x1b[0m"));
    }
}

#[test]
fn slice_by_column_handles_wide_characters_and_strict_boundaries() {
    assert_eq!(slice_by_column("ab界cd", 1, 3, true), "b界");
    assert_eq!(slice_by_column("a界b", 1, 1, false), "界");
    assert_eq!(slice_by_column("a界b", 1, 1, true), "");
}

#[test]
fn slice_with_width_preserves_pending_ansi_before_visible_content() {
    let sliced = slice_with_width("\x1b[31mabc\x1b[0m", 1, 2, true);

    assert_eq!(sliced.width, 2);
    assert_eq!(sliced.text, "\x1b[31mbc");
}

#[test]
fn extract_segments_carries_active_styles_into_after_segment() {
    let extracted = extract_segments("\x1b[3mhello world\x1b[23m", 2, 5, 5, true);

    assert_eq!(extracted.before, "\x1b[3mhe");
    assert_eq!(extracted.before_width, 2);
    assert_eq!(extracted.after, "\x1b[3m worl");
    assert_eq!(extracted.after_width, 5);
}

#[test]
fn extract_segments_respects_strict_after_boundaries_for_wide_characters() {
    let extracted = extract_segments("ab界cd", 2, 2, 1, true);

    assert_eq!(extracted.before, "ab");
    assert_eq!(extracted.before_width, 2);
    assert_eq!(extracted.after, "");
    assert_eq!(extracted.after_width, 0);
}
