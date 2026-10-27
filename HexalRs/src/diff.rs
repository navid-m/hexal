//! Unified line diff used by the sidebar's Diff action.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const CONTEXT: usize = 3;
const MAX_LCS_LINES: usize = 4000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Equal,
    Delete,
    Insert,
}

struct Operation<'a> {
    kind: Kind,
    old_index: usize,
    new_index: usize,
    text: &'a str,
}

pub fn create(old_label: &str, old_text: &str, new_label: &str, new_text: &str) -> String {
    let old_lines = split_lines(old_text);
    let new_lines = split_lines(new_text);
    let edits = myers(&old_lines, &new_lines);

    let mut operations = Vec::with_capacity(edits.len());
    let mut old_index = 0usize;
    let mut new_index = 0usize;
    for edit in edits {
        match edit {
            Kind::Equal => {
                operations.push(Operation {
                    kind: edit,
                    old_index,
                    new_index,
                    text: old_lines[old_index],
                });
                old_index += 1;
                new_index += 1;
            }
            Kind::Delete => {
                operations.push(Operation {
                    kind: edit,
                    old_index,
                    new_index,
                    text: old_lines[old_index],
                });
                old_index += 1;
            }
            Kind::Insert => {
                operations.push(Operation {
                    kind: edit,
                    old_index,
                    new_index,
                    text: new_lines[new_index],
                });
                new_index += 1;
            }
        }
    }

    let mut output = String::new();
    output.push_str("--- ");
    output.push_str(old_label);
    output.push('\n');
    output.push_str("+++ ");
    output.push_str(new_label);
    output.push('\n');
    append_hunks(&mut output, &operations);
    output
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            let end = if index > start && text.as_bytes()[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push(&text[start..end]);
            start = index + 1;
        }
    }
    if start < text.len() {
        let mut end = text.len();
        if end > start && text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(&text[start..end]);
    }
    lines
}

/// Myers' O(ND) diff. Falls back to delete-all/insert-all for huge inputs.
fn myers(old: &[&str], new: &[&str]) -> Vec<Kind> {
    if old.len() * new.len() > MAX_LCS_LINES * MAX_LCS_LINES {
        let mut edits = Vec::with_capacity(old.len() + new.len());
        edits.extend(std::iter::repeat(Kind::Delete).take(old.len()));
        edits.extend(std::iter::repeat(Kind::Insert).take(new.len()));
        return edits;
    }
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m) as usize;
    let offset = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    let mut trace: Vec<Vec<isize>> = Vec::new();

    let mut found = false;
    for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = true;
                break;
            }
            k += 2;
        }
        if found {
            break;
        }
    }

    let mut edits = Vec::new();
    let mut x = n;
    let mut y = m;
    let mut d = trace.len() as isize - 1;
    while d > 0 {
        let v = &trace[d as usize];
        let k = x - y;
        let index = (k + offset) as usize;
        let previous_k = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = v[(previous_k + offset) as usize];
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            edits.push(Kind::Equal);
            x -= 1;
            y -= 1;
        }
        if x == previous_x {
            edits.push(Kind::Insert);
            y -= 1;
        } else {
            edits.push(Kind::Delete);
            x -= 1;
        }
        d -= 1;
    }
    while x > 0 && y > 0 {
        edits.push(Kind::Equal);
        x -= 1;
        y -= 1;
    }
    while x > 0 {
        edits.push(Kind::Delete);
        x -= 1;
    }
    while y > 0 {
        edits.push(Kind::Insert);
        y -= 1;
    }
    edits.reverse();
    edits
}

fn append_hunks(output: &mut String, operations: &[Operation<'_>]) {
    let mut changes = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        if operation.kind != Kind::Equal {
            changes.push(index);
        }
    }
    if changes.is_empty() {
        return;
    }

    let mut hunks = Vec::new();
    let mut start = changes[0].saturating_sub(CONTEXT);
    let mut end = (changes[0] + CONTEXT).min(operations.len() - 1);
    for &change in &changes[1..] {
        let next_start = change.saturating_sub(CONTEXT);
        let next_end = (change + CONTEXT).min(operations.len() - 1);
        if next_start <= end + 1 {
            end = end.max(next_end);
        } else {
            hunks.push((start, end));
            start = next_start;
            end = next_end;
        }
    }
    hunks.push((start, end));

    for (start, end) in hunks {
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        for operation in &operations[start..=end] {
            match operation.kind {
                Kind::Equal => {
                    old_count += 1;
                    new_count += 1;
                }
                Kind::Delete => old_count += 1,
                Kind::Insert => new_count += 1,
            }
        }
        let old_start = operations[start].old_index + 1;
        let new_start = operations[start].new_index + 1;
        output.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for operation in &operations[start..=end] {
            let marker = match operation.kind {
                Kind::Equal => ' ',
                Kind::Delete => '-',
                Kind::Insert => '+',
            };
            output.push(marker);
            output.push_str(operation.text);
            output.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_diff() {
        let diff = create("a", "one\ntwo\nthree\n", "b", "one\nonly\nthree\n");
        assert!(diff.contains("--- a"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+only"));
    }
}
