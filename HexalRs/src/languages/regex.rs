//! A small regular-expression matcher for grammar `linePatterns`.
//!
//! The edit grammar schema historically used .NET regular expressions. Only
//! the subset needed by line-prefix highlighting is implemented: anchors,
//! literals, escapes, character classes, `.`, groups, alternation and greedy
//! quantifiers. Matching is byte-oriented so reported offsets can be used
//! directly against the editor's UTF-8 buffers.

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Byte(u8),
    Any,
    Class {
        negated: bool,
        ranges: Vec<(u8, u8)>,
    },
    Start,
    End,
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: usize,
    },
}

#[derive(Clone, Debug)]
pub struct Regex {
    root: Node,
    case_insensitive: bool,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, String> {
        let mut parser = Parser {
            bytes: pattern.as_bytes(),
            position: 0,
            case_insensitive: false,
        };
        let root = parser.parse_alternation()?;
        if parser.position != parser.bytes.len() {
            return Err(format!(
                "unexpected trailing input at byte {}",
                parser.position
            ));
        }
        Ok(Regex {
            root,
            case_insensitive: parser.case_insensitive,
        })
    }

    /// Returns the byte range of the first non-empty match, mirroring
    /// `Regex.Match` usage in the reference implementation.
    pub fn find(&self, text: &[u8]) -> Option<(usize, usize)> {
        for start in 0..=text.len() {
            let mut ends = Vec::new();
            match_node(&self.root, text, start, self.case_insensitive, &mut ends);
            if let Some(end) = ends.into_iter().filter(|end| *end > start).max() {
                return Some((start, end));
            }
        }
        None
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    case_insensitive: bool,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let value = self.peek();
        if value.is_some() {
            self.position += 1;
        }
        value
    }

    fn parse_alternation(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.parse_sequence()?];
        while self.peek() == Some(b'|') {
            self.bump();
            branches.push(self.parse_sequence()?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().unwrap_or(Node::Empty)
        } else {
            Node::Alt(branches)
        })
    }

    fn parse_sequence(&mut self) -> Result<Node, String> {
        let mut nodes = Vec::new();
        while let Some(byte) = self.peek() {
            if byte == b'|' || byte == b')' {
                break;
            }
            let node = self.parse_atom()?;
            let node = self.parse_quantifier(node);
            nodes.push(node);
        }
        Ok(match nodes.len() {
            0 => Node::Empty,
            1 => nodes.pop().unwrap_or(Node::Empty),
            _ => Node::Seq(nodes),
        })
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        let byte = self
            .bump()
            .ok_or_else(|| "unexpected end of pattern".to_string())?;
        match byte {
            b'^' => Ok(Node::Start),
            b'$' => Ok(Node::End),
            b'.' => Ok(Node::Any),
            b'(' => {
                if self.peek() == Some(b'?') {
                    self.bump();
                    let marker = self.bump();
                    match marker {
                        Some(b':') => {}
                        Some(b'i') => {
                            self.case_insensitive = true;
                            if self.peek() == Some(b')') {
                                // `(?i)` applies to the remainder of the
                                // pattern, matching .NET's inline option.
                                self.bump();
                                return Ok(Node::Empty);
                            }
                            if self.bump() != Some(b':') {
                                return Err("expected ':' after (?i".into());
                            }
                        }
                        Some(b'=') | Some(b'!') => {
                            return Err("lookahead is not supported".into());
                        }
                        _ => return Err("unsupported group prefix".into()),
                    }
                }
                let inner = self.parse_alternation()?;
                if self.bump() != Some(b')') {
                    return Err("missing ')'".into());
                }
                Ok(inner)
            }
            b')' => Err("unexpected ')'".into()),
            b'[' => self.parse_class(),
            b'\\' => self.parse_escape(),
            b'*' | b'+' | b'?' | b'}' => Err("unexpected quantifier".into()),
            other => Ok(Node::Byte(other)),
        }
    }

    fn parse_escape(&mut self) -> Result<Node, String> {
        let byte = self
            .bump()
            .ok_or_else(|| "unterminated escape".to_string())?;
        match byte {
            b'd' => Ok(Node::Class {
                negated: false,
                ranges: vec![(b'0', b'9')],
            }),
            b'D' => Ok(Node::Class {
                negated: true,
                ranges: vec![(b'0', b'9')],
            }),
            b'w' => Ok(Node::Class {
                negated: false,
                ranges: vec![(b'a', b'z'), (b'A', b'Z'), (b'0', b'9'), (b'_', b'_')],
            }),
            b'W' => Ok(Node::Class {
                negated: true,
                ranges: vec![(b'a', b'z'), (b'A', b'Z'), (b'0', b'9'), (b'_', b'_')],
            }),
            b's' => Ok(Node::Class {
                negated: false,
                ranges: vec![
                    (b' ', b' '),
                    (b'\t', b'\t'),
                    (b'\r', b'\r'),
                    (b'\n', b'\n'),
                    (0x0B, 0x0C),
                ],
            }),
            b'S' => Ok(Node::Class {
                negated: true,
                ranges: vec![
                    (b' ', b' '),
                    (b'\t', b'\t'),
                    (b'\r', b'\r'),
                    (b'\n', b'\n'),
                    (0x0B, 0x0C),
                ],
            }),
            b'b' => Ok(Node::Empty),
            b'B' => Ok(Node::Empty),
            b'n' => Ok(Node::Byte(b'\n')),
            b'r' => Ok(Node::Byte(b'\r')),
            b't' => Ok(Node::Byte(b'\t')),
            other => Ok(Node::Byte(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Node, String> {
        let mut negated = false;
        if self.peek() == Some(b'^') {
            self.bump();
            negated = true;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let byte = self
                .bump()
                .ok_or_else(|| "unterminated character class".to_string())?;
            if byte == b']' && !first {
                break;
            }
            first = false;
            let start = if byte == b'\\' {
                match self.bump() {
                    Some(b'd') => {
                        ranges.push((b'0', b'9'));
                        if self.peek() == Some(b'-')
                            && self.bytes.get(self.position + 1) != Some(&b']')
                        {
                            self.bump();
                            let end = self.bump().unwrap_or(b'0');
                            ranges.push((end, end));
                        }
                        continue;
                    }
                    Some(b'w') => {
                        ranges.push((b'a', b'z'));
                        ranges.push((b'A', b'Z'));
                        ranges.push((b'0', b'9'));
                        ranges.push((b'_', b'_'));
                        continue;
                    }
                    Some(b's') => {
                        ranges.push((b' ', b' '));
                        ranges.push((b'\t', b'\t'));
                        ranges.push((b'\r', b'\r'));
                        ranges.push((b'\n', b'\n'));
                        continue;
                    }
                    Some(other) => other,
                    None => return Err("unterminated escape in character class".into()),
                }
            } else {
                byte
            };

            if self.peek() == Some(b'-') && self.bytes.get(self.position + 1).copied() != Some(b']')
            {
                self.bump();
                let end = self
                    .bump()
                    .ok_or_else(|| "unterminated range".to_string())?;
                ranges.push((start, end));
            } else {
                ranges.push((start, start));
            }
        }
        Ok(Node::Class { negated, ranges })
    }

    fn parse_quantifier(&mut self, node: Node) -> Node {
        match self.peek() {
            Some(b'*') => {
                self.bump();
                self.skip_lazy();
                Node::Repeat {
                    node: Box::new(node),
                    min: 0,
                    max: usize::MAX,
                }
            }
            Some(b'+') => {
                self.bump();
                self.skip_lazy();
                Node::Repeat {
                    node: Box::new(node),
                    min: 1,
                    max: usize::MAX,
                }
            }
            Some(b'?') => {
                self.bump();
                self.skip_lazy();
                Node::Repeat {
                    node: Box::new(node),
                    min: 0,
                    max: 1,
                }
            }
            Some(b'{') => {
                let saved = self.position;
                self.bump();
                let mut min = 0usize;
                let mut digits = 0;
                while let Some(byte) = self.peek() {
                    if !byte.is_ascii_digit() {
                        break;
                    }
                    min = min.saturating_mul(10) + (byte - b'0') as usize;
                    digits += 1;
                    self.bump();
                }
                if digits == 0 {
                    self.position = saved;
                    return node;
                }
                let mut max = min;
                if self.peek() == Some(b',') {
                    self.bump();
                    let mut digits = 0;
                    let mut value = 0usize;
                    while let Some(byte) = self.peek() {
                        if !byte.is_ascii_digit() {
                            break;
                        }
                        value = value.saturating_mul(10) + (byte - b'0') as usize;
                        digits += 1;
                        self.bump();
                    }
                    max = if digits == 0 { usize::MAX } else { value };
                }
                if self.peek() == Some(b'}') {
                    self.bump();
                    self.skip_lazy();
                    Node::Repeat {
                        node: Box::new(node),
                        min,
                        max,
                    }
                } else {
                    self.position = saved;
                    node
                }
            }
            _ => node,
        }
    }

    fn skip_lazy(&mut self) {
        if self.peek() == Some(b'?') {
            self.bump();
        }
    }
}

fn ascii_eq(left: u8, right: u8, insensitive: bool) -> bool {
    if insensitive {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

fn class_matches(class: &Node, byte: u8, insensitive: bool) -> bool {
    let Node::Class { negated, ranges } = class else {
        return false;
    };
    let mut found = ranges
        .iter()
        .any(|(start, end)| byte >= *start && byte <= *end);
    if !found && insensitive {
        let lower = byte.to_ascii_lowercase();
        let upper = byte.to_ascii_uppercase();
        found = ranges.iter().any(|(start, end)| {
            (lower >= *start && lower <= *end) || (upper >= *start && upper <= *end)
        });
    }
    if *negated { !found } else { found }
}

fn match_node(node: &Node, text: &[u8], position: usize, insensitive: bool, out: &mut Vec<usize>) {
    match node {
        Node::Empty => out.push(position),
        Node::Byte(byte) => {
            if let Some(current) = text.get(position) {
                if ascii_eq(*current, *byte, insensitive) {
                    out.push(position + 1);
                }
            }
        }
        Node::Any => {
            if let Some(current) = text.get(position) {
                if *current != b'\n' {
                    out.push(position + 1);
                }
            }
        }
        Node::Class { .. } => {
            if let Some(current) = text.get(position) {
                if class_matches(node, *current, insensitive) {
                    out.push(position + 1);
                }
            }
        }
        Node::Start => {
            if position == 0 {
                out.push(position);
            }
        }
        Node::End => {
            if position == text.len() {
                out.push(position);
            }
        }
        Node::Seq(nodes) => {
            let mut positions = vec![position];
            for node in nodes {
                let mut next = Vec::new();
                for pos in positions {
                    match_node(node, text, pos, insensitive, &mut next);
                }
                next.sort_unstable();
                next.dedup();
                if next.is_empty() {
                    return;
                }
                positions = next;
            }
            out.extend(positions);
        }
        Node::Alt(branches) => {
            for branch in branches {
                match_node(branch, text, position, insensitive, out);
            }
        }
        Node::Repeat { node, min, max } => {
            // All reachable lengths; bounded by the input so zero-width
            // repeats cannot loop forever.
            let mut current = vec![position];
            if *min == 0 {
                out.push(position);
            }
            let ceiling = (*max).min(text.len() + 1);
            for count in 1..=ceiling {
                let mut next = Vec::new();
                for pos in &current {
                    match_node(node, text, *pos, insensitive, &mut next);
                }
                next.sort_unstable();
                next.dedup();
                next.retain(|pos| *pos > position || *min == usize::MAX);
                // Prevent zero-width repetitions from cycling.
                next.retain(|pos| !current.contains(pos));
                if next.is_empty() {
                    break;
                }
                current = next;
                if count >= *min {
                    out.extend(current.iter().copied());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Regex;

    #[test]
    fn matches_diff_prefixes() {
        let added = Regex::new("^\\+.*").unwrap();
        assert_eq!(added.find(b"+ hello"), Some((0, 7)));
        assert_eq!(added.find(b"- hello"), None);
        let removed = Regex::new("^-.*").unwrap();
        assert_eq!(removed.find(b"- hello"), Some((0, 7)));
    }

    #[test]
    fn matches_dotenv_assignment() {
        let regex = Regex::new("^(?:export\\s+)?[A-Za-z_][A-Za-z0-9_.]*\\s*=").unwrap();
        assert_eq!(regex.find(b"FOO=bar"), Some((0, 4)));
        assert_eq!(regex.find(b"export FOO.BAR = 1"), Some((0, 16)));
        assert_eq!(regex.find(b"# FOO=bar"), None);
    }

    #[test]
    fn matches_case_insensitive_commit_subject() {
        let regex = Regex::new("^(?i)[a-z][a-z0-9_-]*(\\([^)]*\\))?:").unwrap();
        assert!(regex.find(b"feat(parser): add").is_some());
        assert!(regex.find(b"Fix: bug").is_some());
        assert!(regex.find(b"not a subject").is_none());
    }
}
