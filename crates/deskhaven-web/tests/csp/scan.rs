//! A tiny HTML start-tag tokenizer, enough to find what a strict CSP blocks.
//! It understands quoted attribute values (so `>` inside one does not end the
//! tag) and skips comments, doctypes and end tags.

#[derive(Debug, PartialEq, Eq)]
pub struct Tag {
    /// Lower-cased.
    pub name: String,
    /// Lower-cased names; values as written.
    pub attrs: Vec<(String, String)>,
}

impl Tag {
    fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|(n, _)| n == name)
    }
}

/// What in `html` a CSP with `script-src 'self'; style-src 'self'` blocks.
pub fn csp_violations(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for tag in start_tags(html) {
        let name = &tag.name;
        if name == "script" && !tag.has_attr("src") {
            found.push(format!("inline <script>: {tag:?}"));
        }
        if name == "style" {
            found.push(format!("<style> element: {tag:?}"));
        }
        for (attr, value) in &tag.attrs {
            let is_js_url = value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("javascript:");
            if attr == "style" {
                found.push(format!("style attribute: {tag:?}"));
            } else if attr.starts_with("on") {
                found.push(format!("event handler {attr}: {tag:?}"));
            } else if is_js_url {
                found.push(format!("javascript: URL in {attr}: {tag:?}"));
            }
        }
    }
    found
}

pub fn start_tags(html: &str) -> Vec<Tag> {
    let chars: Vec<char> = html.chars().collect();
    let mut tags = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        if starts_with(&chars, i, "<!--") {
            i = find(&chars, i + 4, "-->").map_or(chars.len(), |end| end + 3);
        } else if chars.get(i + 1).is_some_and(char::is_ascii_alphabetic) {
            let (tag, next) = read_tag(&chars, i + 1);
            tags.push(tag);
            i = next;
        } else {
            // End tag, doctype, processing instruction or a stray `<`.
            i += 1;
        }
    }
    tags
}

fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(k, c)| chars.get(at + k) == Some(&c))
}

fn find(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..chars.len()).find(|&at| starts_with(chars, at, pat))
}

fn is_name_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '/' | '>' | '=')
}

fn take_while(chars: &[char], mut i: usize, pred: impl Fn(char) -> bool) -> (String, usize) {
    let start = i;
    while i < chars.len() && pred(chars[i]) {
        i += 1;
    }
    (chars[start..i].iter().collect(), i)
}

/// `i` is just after `<`. Returns the tag and the index after its closing `>`.
fn read_tag(chars: &[char], i: usize) -> (Tag, usize) {
    let (name, mut i) = take_while(chars, i, |c| !is_name_end(c));
    let mut attrs = Vec::new();
    loop {
        (_, i) = take_while(chars, i, |c| c.is_whitespace() || c == '/');
        match chars.get(i) {
            None => break,
            Some('>') => {
                i += 1;
                break;
            }
            Some(_) => {}
        }
        let (attr, next) = take_while(chars, i, |c| !is_name_end(c));
        // A lone `=` at the start of a name would never advance; consume it.
        i = if next == i { i + 1 } else { next };
        (_, i) = take_while(chars, i, char::is_whitespace);
        let mut value = String::new();
        if chars.get(i) == Some(&'=') {
            (_, i) = take_while(chars, i + 1, char::is_whitespace);
            (value, i) = read_value(chars, i);
        }
        if !attr.is_empty() {
            attrs.push((attr.to_ascii_lowercase(), value));
        }
    }
    let tag = Tag {
        name: name.to_ascii_lowercase(),
        attrs,
    };
    (tag, i)
}

fn read_value(chars: &[char], i: usize) -> (String, usize) {
    match chars.get(i) {
        Some(&quote) if quote == '"' || quote == '\'' => {
            let (value, end) = take_while(chars, i + 1, |c| c != quote);
            (value, end + 1)
        }
        _ => take_while(chars, i, |c| !c.is_whitespace() && c != '>'),
    }
}
