//! Convert standard Markdown to Slack mrkdwn format.
//!
//! Slack uses its own "mrkdwn" syntax that differs from standard Markdown:
//! - Bold: `*text*` (not `**text**`)
//! - Italic: `_text_` (same)
//! - Strikethrough: `~text~` (not `~~text~~`)
//! - Code: `` `code` `` (same)
//! - Code blocks: ``` (same)
//! - Links: `<url|text>` (not `[text](url)`)
//! - Headings: not supported — convert to bold text

/// Convert Markdown text to Slack mrkdwn format.
pub fn markdown_to_mrkdwn(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_code_block = false;
    let mut in_inline_code = false;
    let mut line_start = true;

    while let Some(c) = chars.next() {
        // Track code blocks — don't convert inside them
        if c == '`' && chars.peek() == Some(&'`') {
            let next = chars.next(); // second `
            if chars.peek() == Some(&'`') {
                chars.next(); // third `
                in_code_block = !in_code_block;
                output.push_str("```");
                continue;
            }
            // Two backticks — put them back
            output.push('`');
            if let Some(n) = next {
                output.push(n);
            }
            continue;
        }

        if in_code_block {
            output.push(c);
            line_start = c == '\n';
            continue;
        }

        // Inline code — don't convert inside
        if c == '`' {
            in_inline_code = !in_inline_code;
            output.push(c);
            continue;
        }

        if in_inline_code {
            output.push(c);
            continue;
        }

        // Headings at line start: # Heading → *Heading*
        if line_start && c == '#' {
            while chars.peek() == Some(&'#') {
                chars.next();
            }
            // Skip space after #
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            // Collect rest of line
            let mut heading = String::new();
            for hc in chars.by_ref() {
                if hc == '\n' {
                    break;
                }
                heading.push(hc);
            }
            let heading = heading.trim_end();
            output.push_str(&format!("*{}*\n", heading));
            line_start = true;
            continue;
        }

        // Bold: **text** → *text*
        if c == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second *
            output.push('*');
            continue;
        }

        // Strikethrough: ~~text~~ → ~text~
        if c == '~' && chars.peek() == Some(&'~') {
            chars.next(); // consume second ~
            output.push('~');
            continue;
        }

        // Links: [text](url) → <url|text>
        if c == '[' {
            let mut text = String::new();
            let mut found_close = false;
            // Collect link text
            for lc in chars.by_ref() {
                if lc == ']' {
                    found_close = true;
                    break;
                }
                text.push(lc);
            }
            if found_close && chars.peek() == Some(&'(') {
                chars.next(); // consume (
                let mut url = String::new();
                for uc in chars.by_ref() {
                    if uc == ')' {
                        break;
                    }
                    url.push(uc);
                }
                output.push_str(&format!("<{}|{}>", url, text));
            } else {
                // Not a link, output as-is
                output.push('[');
                output.push_str(&text);
                if found_close {
                    output.push(']');
                }
            }
            continue;
        }

        line_start = c == '\n';
        output.push(c);
    }

    output
}
