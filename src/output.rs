/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::sync::atomic::{AtomicBool, Ordering};

use color_eyre::eyre::Result;

use crate::{git::PreparedCommit, message::MessageSection};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Set quiet mode globally. When quiet, only essential output is printed.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

pub fn output(icon: &str, text: &str) -> Result<()> {
    if is_quiet() {
        return Ok(());
    }

    let term = console::Term::stdout();

    let bullet = format!("  {}  ", icon);
    let indent = console::measure_text_width(&bullet);
    let indent_string = " ".repeat(indent);
    let options = textwrap::Options::new((term.size().1 as usize) - indent * 2)
        .initial_indent(&bullet)
        .subsequent_indent(&indent_string)
        .break_words(false)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);

    term.write_line(&textwrap::wrap(text.trim(), &options).join("\n"))?;
    Ok(())
}

/// Print essential output that is always shown, even in quiet mode.
/// Used for PR URLs, numbers, and other machine-relevant data.
pub fn output_essential(text: &str) -> Result<()> {
    println!("{}", text);
    Ok(())
}

pub fn write_commit_title(prepared_commit: &PreparedCommit) -> Result<()> {
    if is_quiet() {
        return Ok(());
    }

    let term = console::Term::stdout();
    term.write_line(&format!(
        "{} {}",
        console::style(&prepared_commit.short_id).italic(),
        console::style(
            prepared_commit
                .message
                .get(&MessageSection::Title)
                .map(|s| &s[..])
                .unwrap_or("(untitled)"),
        )
        .yellow()
    ))?;
    Ok(())
}
