//! Tiny CSS subset parser + selector matcher for dictionary stylesheets.
//!
//! Aims to cover what real MDX stylesheets (LDOCE5, OALD, Cambridge,
//! Merriam-Webster, hycd_3rd) actually use:
//!
//! - simple selectors: `tag`, `.class`, `#id`, `tag.class`
//! - compound selectors via descendant combinator (whitespace)
//! - selector lists separated by commas (`a, b { ... }`)
//! - declarations: `name: value;` pairs inside `{ ... }`
//! - block comments `/* ... */`
//!
//! Not supported: pseudo-classes, `::before`, attribute selectors,
//! adjacent/child combinators, media queries, `@import`, calc(), etc.
//! Unrecognized constructs are skipped so the rest of the file still
//! parses.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    /// `a, b, c { ... }` produces three Selectors sharing one decl list.
    pub selectors: Vec<Selector>,
    pub decls: Vec<Declaration>,
}

#[derive(Debug, Clone)]
pub struct Selector {
    /// Descendant chain — innermost (target element) is last.
    pub parts: Vec<Compound>,
}

#[derive(Debug, Clone, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub value: String,
}

/// One element's context in the open tree, used during matching.
#[derive(Debug, Clone)]
pub struct ElementCtx<'a> {
    pub tag: &'a str,
    pub classes: &'a [String],
    pub id: Option<&'a str>,
}

impl Stylesheet {
    pub fn parse(input: &str) -> Self {
        let cleaned = strip_comments(input);
        let mut rules = Vec::new();
        let mut i = 0;
        let bytes = cleaned.as_bytes();
        while i < bytes.len() {
            skip_ws(&cleaned, &mut i);
            if i >= bytes.len() {
                break;
            }
            // Skip at-rules: `@media ... { ... }` or `@import ...;`
            if bytes[i] == b'@' {
                if !skip_at_rule(&cleaned, &mut i) {
                    break;
                }
                continue;
            }
            let sel_start = i;
            while i < bytes.len() && bytes[i] != b'{' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let selectors_text = &cleaned[sel_start..i];
            i += 1; // consume {
            let body_start = i;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            let body_end = i - 1; // before closing }
            let body = &cleaned[body_start..body_end];
            let selectors = parse_selectors(selectors_text);
            let decls = parse_decls(body);
            if !selectors.is_empty() && !decls.is_empty() {
                rules.push(Rule { selectors, decls });
            }
        }
        Self { rules }
    }

    /// Concatenate another stylesheet's rules onto this one (later rules win).
    pub fn extend(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
    }

    /// Collect all declarations whose selector matches `target` with the
    /// given ancestor chain (oldest first). Later rules override earlier
    /// ones; we deduplicate by property name preserving source order.
    pub fn matching(
        &self,
        target: &ElementCtx<'_>,
        ancestors: &[ElementCtx<'_>],
    ) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|sel| matches_selector(sel, target, ancestors))
            {
                for d in &rule.decls {
                    out.insert(d.name.clone(), d.value.clone());
                }
            }
        }
        out
    }
}

fn matches_selector(sel: &Selector, target: &ElementCtx<'_>, ancestors: &[ElementCtx<'_>]) -> bool {
    let parts = &sel.parts;
    if parts.is_empty() {
        return false;
    }
    // Innermost part must match the target element.
    let target_part = &parts[parts.len() - 1];
    if !compound_matches(target_part, target) {
        return false;
    }
    // Each preceding part must match some ancestor in order (descendant combinator).
    let mut anc_iter = ancestors.iter();
    'outer: for part in parts[..parts.len() - 1].iter() {
        for anc in anc_iter.by_ref() {
            if compound_matches(part, anc) {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

fn compound_matches(c: &Compound, el: &ElementCtx<'_>) -> bool {
    if let Some(tag) = &c.tag
        && !tag.eq_ignore_ascii_case(el.tag)
    {
        return false;
    }
    if let Some(id) = &c.id
        && el.id != Some(id.as_str())
    {
        return false;
    }
    for cls in &c.classes {
        if !el.classes.iter().any(|c| c == cls) {
            return false;
        }
    }
    true
}

fn parse_selectors(text: &str) -> Vec<Selector> {
    text.split(',')
        .filter_map(|chunk| {
            let parts: Vec<Compound> = chunk
                .split_whitespace()
                .filter_map(parse_compound)
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(Selector { parts })
            }
        })
        .collect()
}

fn parse_compound(token: &str) -> Option<Compound> {
    // Strip combinators we don't handle (>, +, ~). For child-combinator-like
    // tokens we just give up on the whole selector by returning None.
    if token == ">" || token == "+" || token == "~" {
        return None;
    }
    // Strip pseudo-classes / elements, e.g. "a:link", "p::before".
    let token = match token.find(':') {
        Some(p) => &token[..p],
        None => token,
    };
    if token.is_empty() {
        return None;
    }

    let mut compound = Compound::default();
    let mut state = ParseState::Tag;
    let mut buf = String::new();

    for c in token.chars() {
        match c {
            '.' | '#' => {
                flush_token(&mut compound, &mut state, &mut buf);
                state = if c == '.' {
                    ParseState::Class
                } else {
                    ParseState::Id
                };
            }
            _ => buf.push(c),
        }
    }
    flush_token(&mut compound, &mut state, &mut buf);

    if compound.tag.is_none() && compound.classes.is_empty() && compound.id.is_none() {
        return None;
    }
    Some(compound)
}

enum ParseState {
    Tag,
    Class,
    Id,
}

fn flush_token(compound: &mut Compound, state: &mut ParseState, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let s = std::mem::take(buf);
    match state {
        ParseState::Tag => {
            // Star selector `*` is treated as "no tag constraint".
            if s != "*" {
                compound.tag = Some(s.to_lowercase());
            }
        }
        ParseState::Class => compound.classes.push(s),
        ParseState::Id => compound.id = Some(s),
    }
}

fn parse_decls(body: &str) -> Vec<Declaration> {
    body.split(';')
        .filter_map(|chunk| {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                return None;
            }
            let colon = chunk.find(':')?;
            let name = chunk[..colon].trim().to_lowercase();
            let value = chunk[colon + 1..].trim().to_string();
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(Declaration { name, value })
        })
        .collect()
}

fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn skip_ws(s: &str, i: &mut usize) {
    let bytes = s.as_bytes();
    while *i < bytes.len() && (bytes[*i] as char).is_whitespace() {
        *i += 1;
    }
}

/// Skip an `@rule`. Returns false if we hit EOF without resolving.
fn skip_at_rule(s: &str, i: &mut usize) -> bool {
    let bytes = s.as_bytes();
    // Find either `;` (declaration-style) or `{` (block-style).
    let mut j = *i;
    while j < bytes.len() && bytes[j] != b';' && bytes[j] != b'{' {
        j += 1;
    }
    if j >= bytes.len() {
        return false;
    }
    if bytes[j] == b';' {
        *i = j + 1;
        return true;
    }
    // Block-style: balance braces.
    j += 1;
    let mut depth = 1;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        j += 1;
    }
    *i = j;
    true
}
