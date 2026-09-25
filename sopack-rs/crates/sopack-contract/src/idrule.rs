//! Id rule templates: `[ids.rules]` string templates (e.g.
//! `"{lang}:{book_code}:{para_key}#{seq}"`) parsed once at load time, then
//! substituted per point into a **uid** string, which `point_id` hashes with
//! `uuid5(NAMESPACE_DNS, uid)` — the Python side's `contract.ID_RULES` +
//! `contract.point_id` (`sopack/contract.py`, now generic and
//! `contract.toml`-driven: `_fmt_uid(template, fields) = template.format(**fields)`).
//!
//! **Every referenced field is required here — including `seq`.**
//! `_fmt_uid` is a bare `str.format(**fields)` with no defaulting of any
//! kind; a missing field raises `KeyError`/`ValueError` regardless of its
//! name. The "`seq` defaults to `0` when the uid has no `#`" behavior lives
//! **one layer up**, in `sopack.format._fields` (mirrored in this
//! workspace's `sopack-format::idfields::fields_from`), which always
//! populates `seq` in the fields map *before* calling `contract.point_id` —
//! so by the time a template substitution runs, `seq` is either genuinely
//! present or the caller's bug, not something this module should paper
//! over. [`IdRule::build_uid`] therefore matches `_fmt_uid` exactly: any
//! missing field is an error, no exceptions.

use std::fmt::Write as _;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::{ContractError, Result};

/// One `[ids.rules]` template, pre-parsed into literal/field runs so each
/// point pays only substitution cost, not re-parsing.
#[derive(Debug, Clone)]
pub struct IdRule {
    pub name: String,
    pub template: String,
    pub namespace: IdNamespace,
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdNamespace {
    Dns,
}

impl IdNamespace {
    fn parse(s: &str) -> Option<IdNamespace> {
        match s {
            "dns" => Some(IdNamespace::Dns),
            _ => None,
        }
    }

    fn uuid_namespace(self) -> Uuid {
        match self {
            IdNamespace::Dns => Uuid::NAMESPACE_DNS,
        }
    }

    fn doc_name(self) -> &'static str {
        match self {
            IdNamespace::Dns => "dns",
        }
    }
}

#[derive(Debug, Clone)]
enum Token {
    Literal(String),
    Field(String),
}

impl IdRule {
    /// Parse *template* (a `[ids.rules]` value) for rule *name*. Errors on
    /// unbalanced braces, an empty `{}` / `{ }` placeholder, or a field name
    /// that is not a plain identifier (`[A-Za-z_][A-Za-z0-9_]*`) — the shapes
    /// every rule in `contracts/e5-large-v1/contract.toml` already satisfies.
    pub fn parse(name: &str, template: &str, namespace_str: &str) -> Result<IdRule> {
        let namespace = IdNamespace::parse(namespace_str).ok_or_else(|| {
            ContractError::InvalidIdRuleTemplate {
                rule: name.to_string(),
                template: template.to_string(),
                reason: format!("unknown [ids] namespace {namespace_str:?} (have: dns)"),
            }
        })?;

        let mut tokens = Vec::new();
        let mut literal = String::new();
        let mut chars = template.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            match c {
                '{' => {
                    if !literal.is_empty() {
                        tokens.push(Token::Literal(std::mem::take(&mut literal)));
                    }
                    let mut field = String::new();
                    loop {
                        match chars.next() {
                            Some((_, '}')) => break,
                            Some((_, fc)) => field.push(fc),
                            None => {
                                return Err(ContractError::InvalidIdRuleTemplate {
                                    rule: name.to_string(),
                                    template: template.to_string(),
                                    reason: "unterminated '{' — missing closing '}'".to_string(),
                                })
                            }
                        }
                    }
                    if field.is_empty()
                        || !field
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_alphabetic() || c == '_')
                            .unwrap_or(false)
                        || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(ContractError::InvalidIdRuleTemplate {
                            rule: name.to_string(),
                            template: template.to_string(),
                            reason: format!(
                                "{field:?} is not a valid field name (expected [A-Za-z_][A-Za-z0-9_]*)"
                            ),
                        });
                    }
                    tokens.push(Token::Field(field));
                }
                '}' => {
                    return Err(ContractError::InvalidIdRuleTemplate {
                        rule: name.to_string(),
                        template: template.to_string(),
                        reason: "unmatched '}' with no preceding '{'".to_string(),
                    });
                }
                other => literal.push(other),
            }
        }
        if !literal.is_empty() {
            tokens.push(Token::Literal(literal));
        }
        if tokens.is_empty() {
            return Err(ContractError::InvalidIdRuleTemplate {
                rule: name.to_string(),
                template: template.to_string(),
                reason: "template is empty".to_string(),
            });
        }

        Ok(IdRule {
            name: name.to_string(),
            template: template.to_string(),
            namespace,
            tokens,
        })
    }

    /// The field names the template references, in order of first
    /// appearance.
    pub fn fields(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .filter_map(|t| match t {
                Token::Field(f) => Some(f.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Build the pre-hash uid string for *fields* (a point's payload, plus
    /// whatever the caller has already merged in — e.g. `seq`). Every
    /// referenced field must be present; there is no defaulting here (see
    /// the module docs for where `seq`'s default actually lives).
    pub fn build_uid(&self, fields: &Map<String, Value>) -> Result<String> {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Literal(s) => out.push_str(s),
                Token::Field(name) => {
                    let value = fields
                        .get(name)
                        .ok_or_else(|| ContractError::MissingIdField {
                            rule: self.name.clone(),
                            field: name.clone(),
                            template: self.template.clone(),
                            available: fields.keys().cloned().collect(),
                        })?;
                    out.push_str(&format_like_python(value, &self.name, name)?);
                }
            }
        }
        Ok(out)
    }

    /// `uuid5(NAMESPACE_DNS, uid)` of [`Self::build_uid`]'s result, as the
    /// canonical hyphenated string Python's `str(uuid.uuid5(...))` produces.
    pub fn point_id(&self, fields: &Map<String, Value>) -> Result<String> {
        let uid = self.build_uid(fields)?;
        Ok(
            Uuid::new_v5(&self.namespace.uuid_namespace(), uid.as_bytes())
                .hyphenated()
                .to_string(),
        )
    }

    /// A human-readable line describing the rule, e.g.
    /// `uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')` — generalises the
    /// Python side's static `ID_RULE_DOC` table generically from the
    /// template, since the mapping from template to doc string is entirely
    /// mechanical (`{x}` → `<x>`).
    pub fn doc(&self) -> String {
        let mut pattern = String::new();
        for token in &self.tokens {
            match token {
                Token::Literal(s) => pattern.push_str(s),
                Token::Field(name) => {
                    let _ = write!(pattern, "<{name}>");
                }
            }
        }
        format!("uuid5({}, '{}')", self.namespace.doc_name(), pattern)
    }
}

/// Render a JSON value the way Python's f-string interpolation (`f"{v}"`)
/// would for a value that came out of `json.loads` — i.e. `str(v)`.
///
/// Payload fields consumed by id rules are documented as str/int
/// (SOPACK-2-FORMAT.md, `[ids.rules]`), so those two cases are exact
/// (`str(x)` for a string, `str(n)` — canonical decimal, no thousands
/// separator, sign only if negative — for an int). `bool`/`null` are
/// included because JSON has no separate boolean/absent-value id fields but
/// a payload could technically carry one under an id field name — Python's
/// `str(True)`/`str(None)` are `"True"`/`"None"`. Floats use Rust's `f64`
/// `Display`, which — like Python's `repr(float)` — is a shortest
/// round-tripping decimal for the overwhelming majority of values; the two
/// are not verified to agree on exponent-notation thresholds for extreme
/// magnitudes, since no id-rule field is documented to ever be a float.
/// Arrays/objects have no sensible single-field id role and are rejected.
fn format_like_python(v: &Value, rule: &str, field: &str) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(u) = n.as_u64() {
                Ok(u.to_string())
            } else if let Some(f) = n.as_f64() {
                Ok(f.to_string())
            } else {
                Ok(n.to_string())
            }
        }
        Value::Bool(b) => Ok(if *b { "True" } else { "False" }.to_string()),
        Value::Null => Ok("None".to_string()),
        Value::Array(_) => Err(ContractError::UnsupportedIdFieldType {
            rule: rule.to_string(),
            field: field.to_string(),
            kind: "array",
        }),
        Value::Object(_) => Err(ContractError::UnsupportedIdFieldType {
            rule: rule.to_string(),
            field: field.to_string(),
            kind: "object",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn sop_plain_builds_expected_uid() {
        let rule = IdRule::parse("sop/plain", "{lang}:{book_code}:{para_key}", "dns").unwrap();
        let f = fields(&[
            ("lang", json!("en")),
            ("book_code", json!("WDYS")),
            ("para_key", json!("1.1")),
        ]);
        assert_eq!(rule.build_uid(&f).unwrap(), "en:WDYS:1.1");
    }

    /// `seq`'s "default to 0" behavior is `sopack-format`'s job
    /// (`idfields::fields_from`), not this module's — see the module docs.
    /// A `fields` map that genuinely lacks `seq` is a caller bug here, same
    /// as any other missing field.
    #[test]
    fn missing_seq_is_an_error_here_no_defaulting() {
        let rule = IdRule::parse("sop/seq", "{lang}:{book_code}:{para_key}#{seq}", "dns").unwrap();
        let f = fields(&[
            ("lang", json!("en")),
            ("book_code", json!("WDYS")),
            ("para_key", json!("1.1")),
        ]);
        let err = rule.build_uid(&f).unwrap_err();
        assert!(matches!(err, ContractError::MissingIdField { ref field, .. } if field == "seq"));
    }

    #[test]
    fn seq_present_as_zero_builds_expected_uid() {
        let rule = IdRule::parse("sop/seq", "{lang}:{book_code}:{para_key}#{seq}", "dns").unwrap();
        let f = fields(&[
            ("lang", json!("en")),
            ("book_code", json!("WDYS")),
            ("para_key", json!("1.1")),
            ("seq", json!(0)),
        ]);
        assert_eq!(rule.build_uid(&f).unwrap(), "en:WDYS:1.1#0");
    }

    #[test]
    fn sop_seq_uses_explicit_seq_when_present() {
        let rule = IdRule::parse("sop/seq", "{lang}:{book_code}:{para_key}#{seq}", "dns").unwrap();
        let f = fields(&[
            ("lang", json!("en")),
            ("book_code", json!("WDYS")),
            ("para_key", json!("1.1")),
            ("seq", json!(3)),
        ]);
        assert_eq!(rule.build_uid(&f).unwrap(), "en:WDYS:1.1#3");
    }

    #[test]
    fn missing_non_seq_field_is_an_error() {
        let rule = IdRule::parse("bible/v1", "bible:{bible}:{osis}", "dns").unwrap();
        let f = fields(&[("bible", json!("kjv"))]);
        let err = rule.build_uid(&f).unwrap_err();
        assert!(matches!(err, ContractError::MissingIdField { .. }));
    }

    #[test]
    fn doc_generalises_python_id_rule_doc_table() {
        let plain = IdRule::parse("sop/plain", "{lang}:{book_code}:{para_key}", "dns").unwrap();
        assert_eq!(plain.doc(), "uuid5(dns, '<lang>:<book_code>:<para_key>')");
        let seq = IdRule::parse("sop/seq", "{lang}:{book_code}:{para_key}#{seq}", "dns").unwrap();
        assert_eq!(
            seq.doc(),
            "uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')"
        );
        let bible = IdRule::parse("bible/v1", "bible:{bible}:{osis}", "dns").unwrap();
        assert_eq!(bible.doc(), "uuid5(dns, 'bible:<bible>:<osis>')");
    }

    #[test]
    fn point_id_matches_python_uuid5() {
        // uuid.uuid5(uuid.NAMESPACE_DNS, "en:WDYS:1.1#0") computed with
        // CPython 3.11 (see the crate's integration conformance test for the
        // full golden set generated from sopack/contract.py directly).
        let rule = IdRule::parse("sop/seq", "{lang}:{book_code}:{para_key}#{seq}", "dns").unwrap();
        let f = fields(&[
            ("lang", json!("en")),
            ("book_code", json!("WDYS")),
            ("para_key", json!("1.1")),
            ("seq", json!(0)),
        ]);
        let id = rule.point_id(&f).unwrap();
        // Recomputed independently below rather than hardcoded twice.
        let expected = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"en:WDYS:1.1#0")
            .hyphenated()
            .to_string();
        assert_eq!(id, expected);
    }

    #[test]
    fn rejects_unbalanced_braces() {
        assert!(IdRule::parse("bad", "{lang:{book_code}", "dns").is_err());
        assert!(IdRule::parse("bad", "{lang}}", "dns").is_err());
    }
}
