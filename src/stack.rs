/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

pub(crate) const STACK_START: &str = "<!-- spr:stack-start -->";
pub(crate) const STACK_END: &str = "<!-- spr:stack-end -->";

/// Sanitize a commit title for safe embedding in stack info.
///
/// Strips HTML comment delimiters, escapes HTML entities, and escapes
/// Markdown link/image syntax to prevent injection.
#[must_use]
pub fn sanitize_title(title: &str) -> String {
    title
        .replace("<!--", "")
        .replace("-->", "")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Wrap stack info content with start/end markers.
#[must_use]
pub fn wrap_with_markers(stack_content: &str) -> String {
    format!("{STACK_START}\n{stack_content}\n{STACK_END}")
}

/// Update a change request body with new stack info.
///
/// - Valid markers: replace content between them.
/// - No markers: append.
/// - Malformed: remove all, append fresh.
/// - Empty body: return just the wrapped stack info.
#[must_use]
pub fn update_body_with_stack(body: &str, stack_info: &str) -> String {
    let wrapped = wrap_with_markers(stack_info);

    if body.trim().is_empty() {
        return wrapped;
    }

    let start_count = body.matches(STACK_START).count();
    let end_count = body.matches(STACK_END).count();

    if start_count == 1
        && end_count == 1
        && let (Some(start_pos), Some(end_pos)) =
            (body.find(STACK_START), body.find(STACK_END))
        && start_pos < end_pos
    {
        let before = body[..start_pos].trim_end();
        let after = body[end_pos + STACK_END.len()..].trim_start();
        let mut result = before.to_owned();
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(&wrapped);
        if !after.is_empty() {
            result.push_str("\n\n");
            result.push_str(after);
        }
        return result;
    }

    if start_count == 0 && end_count == 0 {
        return format!("{}\n\n{wrapped}", body.trim_end());
    }

    log::warn!("Malformed stack markers in change request body; replacing");
    let cleaned = body.replace(STACK_START, "").replace(STACK_END, "");
    format!("{}\n\n{wrapped}", cleaned.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_title_clean() {
        assert_eq!(sanitize_title("Add widget support"), "Add widget support");
    }

    #[test]
    fn test_sanitize_title_html_comment() {
        assert_eq!(
            sanitize_title("Fix <!-- spr:stack-end --> bug"),
            "Fix  spr:stack-end  bug"
        );
    }

    #[test]
    fn test_sanitize_title_html_entities() {
        assert_eq!(
            sanitize_title("Fix <img> & \"quotes\""),
            "Fix &lt;img&gt; &amp; &quot;quotes&quot;"
        );
    }

    #[test]
    fn test_sanitize_title_markdown_links() {
        assert_eq!(
            sanitize_title("[click](https://evil.com)"),
            "\\[click\\]\\(https://evil.com\\)"
        );
    }

    #[test]
    fn test_update_body_empty() {
        let result = update_body_with_stack("", "**Stack:**\n- #1");
        assert!(result.starts_with(STACK_START));
        assert!(!result.starts_with('\n'));
    }

    #[test]
    fn test_update_body_no_markers() {
        let body = "Some PR description";
        let result = update_body_with_stack(body, "**Stack:**\n- #1");
        assert!(result.contains(STACK_START));
        assert!(result.contains(STACK_END));
        assert!(result.contains("Some PR description"));
        assert!(result.contains("**Stack:**\n- #1"));
    }

    #[test]
    fn test_update_body_existing_markers() {
        let body = format!(
            "Description\n\n{STACK_START}\nold stack\n{STACK_END}\n\nFooter",
        );
        let result =
            update_body_with_stack(&body, "**Stack:**\n- #2 (this)\n- #1");
        assert!(result.contains("Description"));
        assert!(result.contains("Footer"));
        assert!(result.contains("- #2 (this)"));
        assert!(!result.contains("old stack"));
    }

    #[test]
    fn test_update_body_malformed_markers() {
        let body = format!(
            "Description\n{STACK_START}\nstale\n{STACK_END}\nextra\n{STACK_START}",
        );
        let result = update_body_with_stack(&body, "**Stack:**\n- #3");
        assert_eq!(result.matches(STACK_START).count(), 1);
        assert_eq!(result.matches(STACK_END).count(), 1);
        assert!(result.contains("- #3"));
    }

    #[test]
    fn test_wrap_with_markers() {
        let result = wrap_with_markers("**Stack:**\n- #1");
        assert!(result.starts_with(STACK_START));
        assert!(result.ends_with(STACK_END));
        assert!(result.contains("**Stack:**\n- #1"));
    }
}
