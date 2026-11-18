//! The folder Claude names in its trust question, read from the rendered frame.

/// Only the displayed headings tolerate terminal cursor gaps, never the path.
pub(super) fn displayed_workspace(rendered: &str) -> Option<&str> {
    let (_, start) = heading_span(rendered, "Accessing workspace:", true)?;
    let frame = &rendered[start..];
    let (end, _) = heading_span(frame, "Quick safety check", false)?;
    let target = frame[..end].trim_start();
    // The path ends at the line boundary before the heading. Spaces before
    // that boundary belong to the pathname; spaces after it are indentation.
    Some(match target.rfind('\n') {
        Some(boundary) => target[..boundary].trim_end_matches(['\r', '\n']),
        None => target,
    })
}

fn heading_span(text: &str, heading: &str, last: bool) -> Option<(usize, usize)> {
    let chars = text
        .char_indices()
        .filter(|(_, ch)| !ch.is_whitespace())
        .collect::<Vec<_>>();
    let needle = heading
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    if needle.is_empty() {
        return None;
    }
    let mut matches = chars
        .windows(needle.len())
        .filter(|window| {
            window
                .iter()
                .zip(&needle)
                .all(|((_, ch), expected)| ch.eq_ignore_ascii_case(expected))
        })
        .map(|window| {
            let (end, ch) = window[window.len() - 1];
            (window[0].0, end + ch.len_utf8())
        });
    if last {
        matches.next_back()
    } else {
        matches.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_displayed_workspace_keeps_its_exact_path() {
        let frame = "Accessing workspace:\n/work/folder  name\nQuick safety check";
        assert_eq!(displayed_workspace(frame), Some("/work/folder  name"));
        assert_eq!(
            displayed_workspace(&frame.replace('\n', "\r\n")),
            Some("/work/folder  name")
        );
        assert_eq!(
            displayed_workspace(
                &frame
                    .replace("Accessing workspace:", "Accessingworkspace:")
                    .replace("Quick safety check", "Quicksafetycheck")
            ),
            Some("/work/folder  name"),
            "headings tolerate missing spaces, the pathname never does"
        );
        assert_eq!(displayed_workspace("Quick safety check"), None);
    }
}
