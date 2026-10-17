//! SPDX licence expressions as crates declare them in `Cargo.toml` (`MIT OR Apache-2.0`,
//! `(MIT OR Apache-2.0) AND Unicode-3.0`, `Apache-2.0 WITH LLVM-exception`), plus the legacy `/`
//! separator older crates use for OR (`MIT/Apache-2.0`).
//!
//! Only what `cargo xtask licences` needs: parse an expression and pick the licences to comply
//! with under a policy.

/// A parsed licence expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// One licence id, with its exception when written `X WITH Y` (kept as `"X WITH Y"`).
    Id(String),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

/// Parse `text`. Operators are case-insensitive; `AND` binds tighter than `OR`.
pub fn parse(text: &str) -> Result<Expr, String> {
    let tokens = tokenize(text);
    let mut p = Parser { tokens: &tokens, at: 0 };
    let expr = p.or()?;
    match p.tokens.get(p.at) {
        None => Ok(expr),
        Some(t) => Err(format!("`{text}`: unexpected `{t}`")),
    }
}

impl Expr {
    /// The licence ids to comply with: for OR the first alternative (in `preference` order, then
    /// as written) that `allowed` accepts as a whole, for AND every operand. `None` when no
    /// alternative is allowed.
    pub fn choose(&self, allowed: &dyn Fn(&str) -> bool, preference: &[&str]) -> Option<Vec<String>> {
        match self {
            Expr::Id(id) => allowed(id).then(|| vec![id.clone()]),
            Expr::And(all) => {
                let mut out = Vec::new();
                for e in all {
                    for id in e.choose(allowed, preference)? {
                        if !out.contains(&id) {
                            out.push(id);
                        }
                    }
                }
                Some(out)
            }
            Expr::Or(any) => {
                let rank = |e: &Expr| match e {
                    Expr::Id(id) => preference.iter().position(|p| p == id).unwrap_or(preference.len()),
                    _ => preference.len(),
                };
                let mut options: Vec<&Expr> = any.iter().collect();
                options.sort_by_key(|e| rank(e)); // stable: ties keep the written order
                options.into_iter().find_map(|e| e.choose(allowed, preference))
            }
        }
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, '(' | ')' | '/') {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            match c {
                '(' | ')' => out.push(c.to_string()),
                '/' => out.push("OR".into()),
                _ => {}
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

struct Parser<'a> {
    tokens: &'a [String],
    at: usize,
}

impl Parser<'_> {
    fn peek_is(&self, op: &str) -> bool {
        self.tokens.get(self.at).is_some_and(|t| t.eq_ignore_ascii_case(op))
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut parts = vec![self.and()?];
        while self.peek_is("OR") {
            self.at += 1;
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Expr::Or(parts) })
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut parts = vec![self.atom()?];
        while self.peek_is("AND") {
            self.at += 1;
            parts.push(self.atom()?);
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Expr::And(parts) })
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let token = self.tokens.get(self.at).ok_or("expression ends early")?;
        self.at += 1;
        if token == "(" {
            let inner = self.or()?;
            if self.tokens.get(self.at).map(String::as_str) != Some(")") {
                return Err("missing `)`".into());
            }
            self.at += 1;
            return Ok(inner);
        }
        if token == ")" || ["AND", "OR", "WITH"].iter().any(|op| token.eq_ignore_ascii_case(op)) {
            return Err(format!("unexpected `{token}`"));
        }
        if self.peek_is("WITH") {
            let exception = self.tokens.get(self.at + 1).ok_or("`WITH` needs an exception")?;
            self.at += 2;
            return Ok(Expr::Id(format!("{token} WITH {exception}")));
        }
        Ok(Expr::Id(token.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Expr {
        Expr::Id(s.into())
    }

    #[test]
    fn parses_cargo_style_expressions() {
        assert_eq!(parse("MIT").unwrap(), id("MIT"));
        assert_eq!(parse("MIT OR Apache-2.0").unwrap(), Expr::Or(vec![id("MIT"), id("Apache-2.0")]));
        assert_eq!(parse("MIT/Apache-2.0").unwrap(), parse("MIT OR Apache-2.0").unwrap());
        assert_eq!(parse("MIT / Apache-2.0").unwrap(), parse("MIT OR Apache-2.0").unwrap());
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            Expr::And(vec![Expr::Or(vec![id("MIT"), id("Apache-2.0")]), id("Unicode-3.0")])
        );
        assert_eq!(parse("Apache-2.0 WITH LLVM-exception or MIT").unwrap(), Expr::Or(vec![id("Apache-2.0 WITH LLVM-exception"), id("MIT")]));
        // AND binds tighter than OR
        assert_eq!(parse("A OR B AND C").unwrap(), Expr::Or(vec![id("A"), Expr::And(vec![id("B"), id("C")])]));
    }

    #[test]
    fn rejects_malformed_expressions() {
        for bad in ["", "MIT OR", "(MIT", "MIT)", "AND MIT", "MIT WITH", "MIT Apache-2.0"] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn chooses_an_allowed_alternative() {
        let allowed = |id: &str| !id.contains("GPL");
        let choose = |s: &str| parse(s).unwrap().choose(&allowed, &["MIT", "Apache-2.0"]);
        assert_eq!(choose("Apache-2.0 OR MIT"), Some(vec!["MIT".to_string()]), "preference wins over written order");
        assert_eq!(choose("Apache-2.0 OR GPL-2.0-only"), Some(vec!["Apache-2.0".to_string()]));
        assert_eq!(choose("Zlib OR BSL-1.0"), Some(vec!["Zlib".to_string()]), "unranked keep the written order");
        assert_eq!(choose("(MIT OR Apache-2.0) AND Unicode-3.0"), Some(vec!["MIT".to_string(), "Unicode-3.0".to_string()]));
        assert_eq!(choose("MIT AND GPL-3.0-only"), None);
        assert_eq!(choose("GPL-3.0-only OR LGPL-2.1-or-later"), None);
    }
}
