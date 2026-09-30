//! A validator for exactly the JSON Schema features `handoff.schema.json`
//! uses (P13 PR 5, maintainer decision of 2026-09-30).
//!
//! **Why not a general JSON Schema library.** `jsonschema` 0.56, which the
//! trial grader used, adds 43 crates and 5.5 MB to the release binary; the
//! handoff schema uses a small, fixed subset of draft 2020-12. This module
//! implements that subset and nothing else: [`CompiledSchema::compile`]
//! refuses any keyword, keyword form or pattern syntax it does not
//! implement, so the skill cannot start using a feature the check would
//! silently ignore. The schema file stays the single source (embedded with
//! `include_str!`), and a differential test holds this validator to
//! `jsonschema`'s verdicts, which stays a development dependency.
//!
//! **The subset.** Annotations (`$schema` 2020-12, `$id`, `title`,
//! `description`); `$defs` at the root and local `$ref`s to them; `type`
//! (a name or a list); `enum`; `const`; `properties`, `required` and
//! `additionalProperties: false`; `items` (one schema), `minItems`,
//! `maxItems` and `uniqueItems`; `minLength` and `maxLength` (Unicode scalar
//! values); `pattern`; integer `minimum` and `maximum`; `allOf`, `anyOf` and
//! `oneOf`; `not` of exactly one `pattern` or one `const`; and `if` with
//! `then` (no `else`).
//!
//! **Patterns** are ECMA-262 regular expressions matched anywhere in the
//! string, run by the `regex` crate without its Unicode tables. The one Perl
//! class the schema uses, `\s`, is written out as the class `jsonschema`
//! translates it to, so the command and the grader's earlier verdicts
//! agree; any other class escape (`\d`, `\w`, `\b`, ...) and `.` are
//! refused at compile time.
//!
//! **Errors** name the instance location as an RFC 6901 pointer built from
//! the schema's member names and array indices, never from the draft's own
//! member names or values.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use regex::Regex;
use serde_json::{Map, Number, Value};

use super::rule::{HandoffFinding, HandoffRule};

/// The one draft this validator implements.
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// The class `jsonschema` 0.56 translates the ECMA-262 `\s` to
/// (`jsonschema-regex`); written out so both read `\s` alike.
const ECMA_SPACE_MEMBERS: &str = r" \t\n\r\x0B\x0C ﻿  ";

/// Guards schema compilation against a reference cycle.
const MAX_SCHEMA_DEPTH: usize = 64;

/// Why the handoff schema could not be compiled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandoffSchemaError {
    /// A keyword this validator does not implement; the name is the
    /// schema's own.
    UnsupportedKeyword {
        /// Where, as a pointer into the schema.
        location: String,
        /// The keyword.
        keyword: String,
    },
    /// A keyword this validator implements, in a form it does not.
    UnsupportedForm {
        /// Where, as a pointer into the schema.
        location: String,
        /// What is unsupported.
        what: &'static str,
    },
    /// A `$ref` that is not `#/$defs/<name>` or names no definition.
    UnresolvedReference {
        /// Where, as a pointer into the schema.
        location: String,
    },
    /// A pattern the regular-expression engine refused, or one using syntax
    /// this validator does not translate.
    UnsupportedPattern {
        /// Where, as a pointer into the schema.
        location: String,
    },
    /// The schema is not JSON.
    NotJson,
}

impl fmt::Display for HandoffSchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedKeyword { location, keyword } => write!(
                formatter,
                "the handoff schema uses the keyword {keyword} at {location:?}, which the check does not implement"
            ),
            Self::UnsupportedForm { location, what } => write!(
                formatter,
                "the handoff schema uses {what} at {location:?}, which the check does not implement"
            ),
            Self::UnresolvedReference { location } => write!(
                formatter,
                "the handoff schema has an unresolved $ref at {location:?}"
            ),
            Self::UnsupportedPattern { location } => write!(
                formatter,
                "the handoff schema has a pattern the check cannot run at {location:?}"
            ),
            Self::NotJson => formatter.write_str("the handoff schema is not JSON"),
        }
    }
}

impl Error for HandoffSchemaError {}

/// A JSON type name of the `type` keyword.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum JsonType {
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
}

impl JsonType {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "null" => Self::Null,
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "string" => Self::String,
            "array" => Self::Array,
            "object" => Self::Object,
            _ => return None,
        })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }

    fn admits(self, value: &Value) -> bool {
        match (self, value) {
            (Self::Null, Value::Null)
            | (Self::Boolean, Value::Bool(_))
            | (Self::Number, Value::Number(_))
            | (Self::String, Value::String(_))
            | (Self::Array, Value::Array(_))
            | (Self::Object, Value::Object(_)) => true,
            (Self::Integer, Value::Number(number)) => is_integer(number),
            _ => false,
        }
    }
}

/// Draft 2020-12 reads a number with a zero fraction as an integer
/// (`jsonschema` does too).
fn is_integer(number: &Number) -> bool {
    number.is_i64() || number.is_u64() || number.as_f64().is_some_and(|value| value.fract() == 0.0)
}

/// The shape a `not` may take.
#[derive(Debug)]
enum Negation {
    /// `not: {pattern}`: text must not match.
    Pattern(Pattern),
    /// `not: {const}`: the value must not be this one.
    Const(Value),
}

/// A compiled `pattern`, with the rule and prose its mismatch reports.
#[derive(Debug)]
struct Pattern {
    regex: Regex,
    rule: HandoffRule,
    message: &'static str,
}

/// One compiled schema object.
#[derive(Debug, Default)]
struct Node {
    reference: Option<usize>,
    types: Option<Vec<JsonType>>,
    enumeration: Option<Vec<Value>>,
    constant: Option<Value>,
    properties: BTreeMap<String, usize>,
    required: Vec<String>,
    closed: bool,
    items: Option<usize>,
    min_items: Option<u64>,
    max_items: Option<u64>,
    unique_items: bool,
    min_length: Option<u64>,
    max_length: Option<u64>,
    pattern: Option<Pattern>,
    minimum: Option<i64>,
    maximum: Option<i64>,
    all_of: Vec<usize>,
    any_of: Vec<usize>,
    one_of: Vec<usize>,
    negation: Option<Negation>,
    condition: Option<(usize, usize)>,
}

/// The compiled handoff schema.
#[derive(Debug)]
pub(crate) struct CompiledSchema {
    nodes: Vec<Node>,
    root: usize,
}

/// One schema error, with what a combinator needs to choose the branch to
/// report.
#[derive(Clone, Debug)]
pub(crate) struct SchemaError {
    pointer: String,
    rule: HandoffRule,
    allowed: Option<Vec<String>>,
    message: &'static str,
    /// Raised by a `const`, which marks a discriminating member.
    from_const: bool,
}

impl SchemaError {
    fn new(pointer: &str, rule: HandoffRule, allowed: Option<Vec<String>>) -> Self {
        Self {
            pointer: pointer.to_owned(),
            rule,
            allowed,
            message: rule.message(),
            from_const: false,
        }
    }

    /// The instance pointer.
    pub(crate) fn pointer(&self) -> &str {
        &self.pointer
    }

    /// The published finding.
    pub(crate) fn into_finding(self) -> HandoffFinding {
        HandoffFinding::at(self.pointer, self.rule, self.allowed, self.message)
    }
}

/// Appends one member name to a pointer, escaped as RFC 6901 says.
pub(crate) fn child_pointer(pointer: &str, member: &str) -> String {
    let mut child = String::with_capacity(pointer.len() + member.len() + 1);
    child.push_str(pointer);
    child.push('/');
    for character in member.chars() {
        match character {
            '~' => child.push_str("~0"),
            '/' => child.push_str("~1"),
            other => child.push(other),
        }
    }
    child
}

/// Appends one array index to a pointer.
pub(crate) fn index_pointer(pointer: &str, index: usize) -> String {
    format!("{pointer}/{index}")
}

struct Compiler<'schema> {
    root: &'schema Map<String, Value>,
    nodes: Vec<Node>,
    definitions: BTreeMap<String, usize>,
}

impl CompiledSchema {
    /// Compiles `schema`, refusing anything outside the implemented subset.
    pub(crate) fn compile(schema: &Value) -> Result<Self, HandoffSchemaError> {
        let root = schema
            .as_object()
            .ok_or_else(|| HandoffSchemaError::UnsupportedForm {
                location: String::new(),
                what: "a root schema that is not an object",
            })?;
        let mut compiler = Compiler {
            root,
            nodes: Vec::new(),
            definitions: BTreeMap::new(),
        };
        let root_id = compiler.compile(schema, "", 0)?;
        // Every definition compiles, used or not, so an unsupported keyword
        // in one that nothing references yet still fails here.
        if let Some(definitions) = root.get("$defs") {
            let definitions =
                definitions
                    .as_object()
                    .ok_or_else(|| HandoffSchemaError::UnsupportedForm {
                        location: "/$defs".to_owned(),
                        what: "$defs that is not an object",
                    })?;
            for name in definitions.keys() {
                compiler.definition(name, "/$defs", 0)?;
            }
        }
        Ok(Self {
            nodes: compiler.nodes,
            root: root_id,
        })
    }

    /// Every error of `instance`, in a stable order.
    pub(crate) fn errors(&self, instance: &Value) -> Vec<SchemaError> {
        let mut errors = Vec::new();
        self.validate(self.root, instance, "", &mut errors);
        errors
    }

    fn is_valid(&self, node: usize, instance: &Value, pointer: &str) -> bool {
        let mut errors = Vec::new();
        self.validate(node, instance, pointer, &mut errors);
        errors.is_empty()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "One pass over the implemented keywords keeps their order in one place"
    )]
    fn validate(&self, id: usize, instance: &Value, pointer: &str, errors: &mut Vec<SchemaError>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if let Some(reference) = node.reference {
            self.validate(reference, instance, pointer, errors);
        }
        if let Some(types) = &node.types
            && !types.iter().any(|kind| kind.admits(instance))
        {
            errors.push(SchemaError::new(
                pointer,
                HandoffRule::WrongType,
                Some(types.iter().map(|kind| kind.name().to_owned()).collect()),
            ));
        }
        if let Some(constant) = &node.constant
            && !json_equal(constant, instance)
        {
            let mut error = SchemaError::new(
                pointer,
                HandoffRule::ValueNotAllowed,
                Some(closed_words(std::slice::from_ref(constant))),
            );
            error.from_const = true;
            errors.push(error);
        }
        if let Some(values) = &node.enumeration
            && !values.iter().any(|value| json_equal(value, instance))
        {
            errors.push(SchemaError::new(
                pointer,
                HandoffRule::ValueNotAllowed,
                Some(closed_words(values)),
            ));
        }
        match instance {
            Value::String(text) => self.validate_string(node, text, pointer, errors),
            Value::Number(number) => validate_number(node, number, pointer, errors),
            Value::Object(members) => self.validate_object(node, members, pointer, errors),
            Value::Array(items) => self.validate_array(node, items, pointer, errors),
            Value::Null | Value::Bool(_) => {}
        }
        for &branch in &node.all_of {
            self.validate(branch, instance, pointer, errors);
        }
        if !node.any_of.is_empty() {
            let outcomes = self.branch_outcomes(&node.any_of, instance, pointer);
            if outcomes.iter().all(|outcome| !outcome.is_empty()) {
                errors.extend(no_branch_matches(pointer, outcomes));
            }
        }
        if !node.one_of.is_empty() {
            let outcomes = self.branch_outcomes(&node.one_of, instance, pointer);
            let valid = outcomes.iter().filter(|outcome| outcome.is_empty()).count();
            if valid == 0 {
                errors.extend(no_branch_matches(pointer, outcomes));
            } else if valid > 1 {
                errors.push(SchemaError::new(pointer, HandoffRule::AmbiguousShape, None));
            }
        }
        match &node.negation {
            Some(Negation::Pattern(pattern)) => {
                if let Value::String(text) = instance
                    && pattern.regex.is_match(text)
                {
                    let mut error = SchemaError::new(pointer, pattern.rule, None);
                    error.message = pattern.message;
                    errors.push(error);
                }
            }
            Some(Negation::Const(value)) if json_equal(value, instance) => {
                let mut error = SchemaError::new(pointer, HandoffRule::ValueForbidden, None);
                error.message = forbidden_value_message(value);
                errors.push(error);
            }
            Some(Negation::Const(_)) | None => {}
        }
        if let Some((condition, consequence)) = node.condition
            && self.is_valid(condition, instance, pointer)
        {
            self.validate(consequence, instance, pointer, errors);
        }
    }

    fn validate_string(
        &self,
        node: &Node,
        text: &str,
        pointer: &str,
        errors: &mut Vec<SchemaError>,
    ) {
        let _ = self;
        let length = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        if node.min_length.is_some_and(|minimum| length < minimum) {
            errors.push(SchemaError::new(pointer, HandoffRule::TextTooShort, None));
        }
        if node.max_length.is_some_and(|maximum| length > maximum) {
            errors.push(SchemaError::new(pointer, HandoffRule::TextTooLong, None));
        }
        if let Some(pattern) = &node.pattern
            && !pattern.regex.is_match(text)
        {
            let mut error = SchemaError::new(pointer, pattern.rule, None);
            error.message = pattern.message;
            errors.push(error);
        }
    }

    fn validate_object(
        &self,
        node: &Node,
        members: &Map<String, Value>,
        pointer: &str,
        errors: &mut Vec<SchemaError>,
    ) {
        for name in &node.required {
            if !members.contains_key(name) {
                errors.push(SchemaError::new(
                    &child_pointer(pointer, name),
                    HandoffRule::MemberMissing,
                    None,
                ));
            }
        }
        if node.closed
            && members
                .keys()
                .any(|name| !node.properties.contains_key(name))
        {
            errors.push(SchemaError::new(
                pointer,
                HandoffRule::MemberUnknown,
                Some(node.properties.keys().cloned().collect()),
            ));
        }
        for (name, &schema) in &node.properties {
            if let Some(value) = members.get(name) {
                self.validate(schema, value, &child_pointer(pointer, name), errors);
            }
        }
    }

    fn validate_array(
        &self,
        node: &Node,
        items: &[Value],
        pointer: &str,
        errors: &mut Vec<SchemaError>,
    ) {
        let count = u64::try_from(items.len()).unwrap_or(u64::MAX);
        if node.min_items.is_some_and(|minimum| count < minimum) {
            errors.push(SchemaError::new(pointer, HandoffRule::TooFewItems, None));
        }
        if node.max_items.is_some_and(|maximum| count > maximum) {
            errors.push(SchemaError::new(pointer, HandoffRule::TooManyItems, None));
        }
        if node.unique_items
            && items.iter().enumerate().any(|(index, item)| {
                items[index + 1..]
                    .iter()
                    .any(|other| json_equal(item, other))
            })
        {
            errors.push(SchemaError::new(pointer, HandoffRule::DuplicateItems, None));
        }
        if let Some(schema) = node.items {
            for (index, item) in items.iter().enumerate() {
                self.validate(schema, item, &index_pointer(pointer, index), errors);
            }
        }
    }

    fn branch_outcomes(
        &self,
        branches: &[usize],
        instance: &Value,
        pointer: &str,
    ) -> Vec<Vec<SchemaError>> {
        branches
            .iter()
            .map(|&branch| {
                let mut errors = Vec::new();
                self.validate(branch, instance, pointer, &mut errors);
                errors
            })
            .collect()
    }
}

fn validate_number(node: &Node, number: &Number, pointer: &str, errors: &mut Vec<SchemaError>) {
    if let Some(minimum) = node.minimum
        && compare(number, minimum) == Some(std::cmp::Ordering::Less)
    {
        errors.push(SchemaError::new(
            pointer,
            HandoffRule::BelowMinimum,
            Some(vec![minimum.to_string()]),
        ));
    }
    if let Some(maximum) = node.maximum
        && compare(number, maximum) == Some(std::cmp::Ordering::Greater)
    {
        errors.push(SchemaError::new(
            pointer,
            HandoffRule::AboveMaximum,
            Some(vec![maximum.to_string()]),
        ));
    }
}

/// Compares a JSON number with an integer bound exactly: integers as
/// integers, a fraction as a float (every bound of the schema is exactly
/// representable).
fn compare(number: &Number, bound: i64) -> Option<std::cmp::Ordering> {
    if let Some(value) = number.as_i64() {
        return Some(i128::from(value).cmp(&i128::from(bound)));
    }
    if let Some(value) = number.as_u64() {
        return Some(i128::from(value).cmp(&i128::from(bound)));
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "Every bound of the handoff schema is far below 2^53"
    )]
    let bound = bound as f64;
    number.as_f64().and_then(|value| value.partial_cmp(&bound))
}

/// JSON equality as JSON Schema defines it: numbers compare by value.
fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            match (left.as_i64(), right.as_i64(), left.as_u64(), right.as_u64()) {
                (Some(left), Some(right), _, _) => left == right,
                (_, _, Some(left), Some(right)) => left == right,
                _ => left.as_f64() == right.as_f64(),
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(name, value)| {
                    right
                        .get(name)
                        .is_some_and(|other| json_equal(value, other))
                })
        }
        _ => left == right,
    }
}

/// The string values of a closed set: the words an agent may write.
fn closed_words(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Whether a branch's errors exclude it outright: its value set or type is
/// wrong at the combinator's own location, or a discriminating member (a
/// `const` of a direct member, such as a citation's `type`) differs.
fn excludes(error: &SchemaError, pointer: &str) -> bool {
    let at_root = error.pointer == pointer
        && matches!(
            error.rule,
            HandoffRule::WrongType | HandoffRule::ValueNotAllowed
        );
    at_root || discriminates(error, pointer)
}

fn discriminates(error: &SchemaError, pointer: &str) -> bool {
    error.from_const
        && error
            .pointer
            .strip_prefix(pointer)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|member| !member.contains('/'))
}

/// What an `anyOf` or `oneOf` that no branch satisfies reports.
///
/// A branch whose errors exclude it (see [`excludes`]) is set aside. When
/// exactly one branch is left, its own errors are the useful ones (a
/// citation whose `type` names the frame branch reports what that branch
/// misses). When every branch is excluded by its value set, type or
/// discriminator, one error lists what is allowed there; otherwise the
/// value matches no shape.
fn no_branch_matches(pointer: &str, outcomes: Vec<Vec<SchemaError>>) -> Vec<SchemaError> {
    let (excluded, viable): (Vec<_>, Vec<_>) = outcomes
        .into_iter()
        .partition(|errors| errors.iter().any(|error| excludes(error, pointer)));
    if let [only] = viable.as_slice() {
        return only.clone();
    }
    if !viable.is_empty() {
        return vec![SchemaError::new(
            pointer,
            HandoffRule::NoMatchingShape,
            None,
        )];
    }
    let reasons: Vec<&SchemaError> = excluded
        .iter()
        .filter_map(|errors| errors.iter().find(|error| excludes(error, pointer)))
        .collect();
    let values: BTreeSet<String> = reasons
        .iter()
        .filter(|error| error.rule == HandoffRule::ValueNotAllowed && error.pointer == pointer)
        .flat_map(|error| error.allowed.iter().flatten().cloned())
        .collect();
    let only_null_or_values = reasons.iter().all(|error| {
        error.pointer == pointer
            && (error.rule == HandoffRule::ValueNotAllowed
                || error.allowed.as_deref() == Some(&["null".to_owned()][..]))
    });
    if only_null_or_values && !values.is_empty() {
        return vec![SchemaError::new(
            pointer,
            HandoffRule::ValueNotAllowed,
            Some(values.into_iter().collect()),
        )];
    }
    if reasons
        .iter()
        .all(|error| error.pointer == pointer && error.rule == HandoffRule::WrongType)
    {
        let types: BTreeSet<String> = reasons
            .iter()
            .flat_map(|error| error.allowed.iter().flatten().cloned())
            .collect();
        return vec![SchemaError::new(
            pointer,
            HandoffRule::WrongType,
            Some(types.into_iter().collect()),
        )];
    }
    if let Some(first) = reasons.first()
        && reasons
            .iter()
            .all(|error| discriminates(error, pointer) && error.pointer == first.pointer)
    {
        let values: BTreeSet<String> = reasons
            .iter()
            .flat_map(|error| error.allowed.iter().flatten().cloned())
            .collect();
        return vec![SchemaError::new(
            &first.pointer,
            HandoffRule::ValueNotAllowed,
            Some(values.into_iter().collect()),
        )];
    }
    vec![SchemaError::new(
        pointer,
        HandoffRule::NoMatchingShape,
        None,
    )]
}

impl Compiler<'_> {
    fn definition(
        &mut self,
        name: &str,
        location: &str,
        depth: usize,
    ) -> Result<usize, HandoffSchemaError> {
        if let Some(&id) = self.definitions.get(name) {
            return Ok(id);
        }
        let schema = self
            .root
            .get("$defs")
            .and_then(|definitions| definitions.get(name))
            .ok_or_else(|| HandoffSchemaError::UnresolvedReference {
                location: location.to_owned(),
            })?;
        let id = self.reserve();
        self.definitions.insert(name.to_owned(), id);
        let node = self.node(schema, &child_pointer("/$defs", name), depth + 1)?;
        self.nodes[id] = node;
        Ok(id)
    }

    fn reserve(&mut self) -> usize {
        self.nodes.push(Node::default());
        self.nodes.len() - 1
    }

    fn compile(
        &mut self,
        schema: &Value,
        location: &str,
        depth: usize,
    ) -> Result<usize, HandoffSchemaError> {
        let id = self.reserve();
        let node = self.node(schema, location, depth)?;
        self.nodes[id] = node;
        Ok(id)
    }

    fn many(
        &mut self,
        value: &Value,
        location: &str,
        depth: usize,
    ) -> Result<Vec<usize>, HandoffSchemaError> {
        let branches = value
            .as_array()
            .filter(|branches| !branches.is_empty())
            .ok_or_else(|| unsupported(location, "a combinator that is not a non-empty list"))?;
        branches
            .iter()
            .enumerate()
            .map(|(index, branch)| self.compile(branch, &index_pointer(location, index), depth))
            .collect()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "One match over the implemented keywords is the subset's definition"
    )]
    fn node(
        &mut self,
        schema: &Value,
        location: &str,
        depth: usize,
    ) -> Result<Node, HandoffSchemaError> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(unsupported(location, "schemas nested this deeply"));
        }
        let members = schema
            .as_object()
            .ok_or_else(|| unsupported(location, "a boolean or non-object schema"))?;
        let mut node = Node::default();
        let mut then_branch = None;
        let mut if_branch = None;
        for (keyword, value) in members {
            let here = child_pointer(location, keyword);
            match keyword.as_str() {
                "$schema" if location.is_empty() && value == DRAFT_2020_12 => {}
                "$id" if location.is_empty() && value.is_string() => {}
                "$defs" if location.is_empty() => {}
                "title" | "description" if value.is_string() => {}
                "$ref" => {
                    let name = value
                        .as_str()
                        .and_then(|reference| reference.strip_prefix("#/$defs/"))
                        .filter(|name| !name.contains('/'))
                        .ok_or_else(|| HandoffSchemaError::UnresolvedReference {
                            location: here.clone(),
                        })?;
                    node.reference = Some(self.definition(name, &here, depth)?);
                }
                "type" => node.types = Some(types(value, &here)?),
                "enum" => {
                    node.enumeration = Some(
                        value
                            .as_array()
                            .filter(|values| !values.is_empty())
                            .cloned()
                            .ok_or_else(|| unsupported(&here, "an enum that is not a list"))?,
                    );
                }
                "const" => node.constant = Some(value.clone()),
                "properties" => {
                    let properties = value
                        .as_object()
                        .ok_or_else(|| unsupported(&here, "properties that are not an object"))?;
                    for (name, member) in properties {
                        let id = self.compile(member, &child_pointer(&here, name), depth + 1)?;
                        node.properties.insert(name.clone(), id);
                    }
                }
                "required" => {
                    node.required = value
                        .as_array()
                        .and_then(|names| {
                            names
                                .iter()
                                .map(|name| name.as_str().map(str::to_owned))
                                .collect::<Option<Vec<_>>>()
                        })
                        .ok_or_else(|| {
                            unsupported(&here, "required that is not a list of names")
                        })?;
                }
                "additionalProperties" if value == &Value::Bool(false) => node.closed = true,
                "items" => node.items = Some(self.compile(value, &here, depth + 1)?),
                "minItems" => node.min_items = Some(count(value, &here)?),
                "maxItems" => node.max_items = Some(count(value, &here)?),
                "uniqueItems" => {
                    node.unique_items = value
                        .as_bool()
                        .ok_or_else(|| unsupported(&here, "uniqueItems that is not a boolean"))?;
                }
                "minLength" => node.min_length = Some(count(value, &here)?),
                "maxLength" => node.max_length = Some(count(value, &here)?),
                "pattern" => node.pattern = Some(pattern(value, &here, false)?),
                "minimum" => node.minimum = Some(bound(value, &here)?),
                "maximum" => node.maximum = Some(bound(value, &here)?),
                "allOf" => node.all_of = self.many(value, &here, depth + 1)?,
                "anyOf" => node.any_of = self.many(value, &here, depth + 1)?,
                "oneOf" => node.one_of = self.many(value, &here, depth + 1)?,
                "not" => node.negation = Some(negation(value, &here)?),
                "if" => if_branch = Some(self.compile(value, &here, depth + 1)?),
                "then" => then_branch = Some(self.compile(value, &here, depth + 1)?),
                "$schema" | "$id" | "$defs" | "title" | "description" | "additionalProperties" => {
                    return Err(unsupported(&here, "this form of an annotation or keyword"));
                }
                other => {
                    return Err(HandoffSchemaError::UnsupportedKeyword {
                        location: location.to_owned(),
                        keyword: other.to_owned(),
                    });
                }
            }
        }
        node.condition = match (if_branch, then_branch) {
            (Some(condition), Some(consequence)) => Some((condition, consequence)),
            (None, None) => None,
            _ => return Err(unsupported(location, "if without then, or then without if")),
        };
        Ok(node)
    }
}

fn unsupported(location: &str, what: &'static str) -> HandoffSchemaError {
    HandoffSchemaError::UnsupportedForm {
        location: location.to_owned(),
        what,
    }
}

fn types(value: &Value, location: &str) -> Result<Vec<JsonType>, HandoffSchemaError> {
    let names: Vec<&Value> = match value {
        Value::Array(names) if !names.is_empty() => names.iter().collect(),
        Value::String(_) => vec![value],
        _ => return Err(unsupported(location, "a type that is not a name or list")),
    };
    names
        .into_iter()
        .map(|name| {
            name.as_str()
                .and_then(JsonType::parse)
                .ok_or_else(|| unsupported(location, "an unknown type name"))
        })
        .collect()
}

fn count(value: &Value, location: &str) -> Result<u64, HandoffSchemaError> {
    value
        .as_u64()
        .ok_or_else(|| unsupported(location, "a count that is not a non-negative integer"))
}

fn bound(value: &Value, location: &str) -> Result<i64, HandoffSchemaError> {
    value
        .as_i64()
        .ok_or_else(|| unsupported(location, "a bound that is not an integer"))
}

fn negation(value: &Value, location: &str) -> Result<Negation, HandoffSchemaError> {
    let members = value
        .as_object()
        .filter(|members| members.len() == 1)
        .ok_or_else(|| unsupported(location, "a not other than one pattern or one const"))?;
    if let Some(forbidden) = members.get("pattern") {
        return Ok(Negation::Pattern(pattern(
            forbidden,
            &child_pointer(location, "pattern"),
            true,
        )?));
    }
    members
        .get("const")
        .cloned()
        .map(Negation::Const)
        .ok_or_else(|| unsupported(location, "a not other than one pattern or one const"))
}

/// Compiles an ECMA-262 `pattern` into a `regex` expression.
fn pattern(value: &Value, location: &str, negated: bool) -> Result<Pattern, HandoffSchemaError> {
    let source = value
        .as_str()
        .ok_or_else(|| unsupported(location, "a pattern that is not a string"))?;
    let translated =
        translate_pattern(source).ok_or_else(|| HandoffSchemaError::UnsupportedPattern {
            location: location.to_owned(),
        })?;
    let regex = Regex::new(&translated).map_err(|_| HandoffSchemaError::UnsupportedPattern {
        location: location.to_owned(),
    })?;
    let (rule, message) = pattern_message(source, negated);
    Ok(Pattern {
        regex,
        rule,
        message,
    })
}

/// Rewrites the ECMA-262 syntax the `regex` crate reads differently, or
/// refuses the pattern (`None`).
///
/// `\s` becomes `jsonschema`'s explicit class; `\uXXXX` and every other
/// escape the `regex` crate reads as ECMA-262 does pass through. Any other
/// Perl class or assertion escape, a back-reference, a group construct
/// (`(?`) and an unescaped `.` are refused, because their meanings differ
/// between the two engines or they need the Unicode tables.
fn translate_pattern(source: &str) -> Option<String> {
    let mut output = String::with_capacity(source.len() + 32);
    let mut characters = source.chars().peekable();
    let mut in_class = false;
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                let escaped = characters.next()?;
                match escaped {
                    's' if in_class => output.push_str(ECMA_SPACE_MEMBERS),
                    's' => {
                        output.push('[');
                        output.push_str(ECMA_SPACE_MEMBERS);
                        output.push(']');
                    }
                    'u' => {
                        output.push_str("\\u");
                        for _ in 0..4 {
                            let digit = characters.next().filter(char::is_ascii_hexdigit)?;
                            output.push(digit);
                        }
                    }
                    '\\' | '/' | '.' | '-' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '^'
                    | '$' | '*' | '+' | '?' | 't' | 'n' | 'r' => {
                        output.push('\\');
                        output.push(escaped);
                    }
                    _ => return None,
                }
            }
            '[' if !in_class => {
                in_class = true;
                output.push('[');
                if characters.peek() == Some(&'^') {
                    characters.next();
                    output.push('^');
                }
            }
            '[' => output.push_str("\\["),
            ']' if in_class => {
                in_class = false;
                output.push(']');
            }
            '.' if !in_class => return None,
            '(' if !in_class && characters.peek() == Some(&'?') => return None,
            // `&&`, `~~` and `--` inside a class are set operations of the
            // `regex` crate and literal characters in ECMA-262.
            '&' | '~' | '-' if in_class && characters.peek() == Some(&character) => return None,
            other => output.push(other),
        }
    }
    (!in_class).then_some(output)
}

/// Fixed prose of each pattern of `handoff.schema.json` ([`pattern_message`]).
const PATTERN_MESSAGES: [(&str, HandoffRule, &str); 17] = [
    (
        "^[A-Za-z0-9 -]+$",
        HandoffRule::FormMismatch,
        "Write the check image's code exactly as you read it: letters, digits, spaces and hyphens only.",
    ),
    (
        "^src_sha256_[0-9a-f]{64}$",
        HandoffRule::FormMismatch,
        "Copy the source_id exactly as VSift printed it: src_sha256_ and 64 lowercase hexadecimal digits.",
    ),
    (
        "^[^\\u0000-\\u001F\\u007F\\u200B-\\u200F\\u202A-\\u202E\\u2066-\\u2069\\uFEFF]*$",
        HandoffRule::TextForbidden,
        "This text holds a raw control, bidirectional or zero-width character; write a hidden character as <U+XXXX>, as display_text does.",
    ),
    (
        "(^|[\\s(\"'`=])(/|~[/\\\\]|\\\\)|[A-Za-z]:[\\\\/]|[A-Za-z][A-Za-z0-9+.-]*://|www[.]",
        HandoffRule::TextForbidden,
        "This text holds an absolute path, a home folder or a link; cite evidence ids, and describe a link without writing its address.",
    ),
    (
        "^ses_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the session id exactly as VSift printed it: ses_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^trv_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the revision id exactly as VSift printed it: trv_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^evd_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the evidence id exactly as VSift printed it: evd_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^op_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "An operation id is op_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^e[1-9][0-9]{0,2}$",
        HandoffRule::FormMismatch,
        "A citation id is e and a number from 1 to 999: e1, e2 and so on.",
    ),
    (
        "^c[1-9][0-9]{0,2}$",
        HandoffRule::FormMismatch,
        "A claim id is c and a number from 1 to 999: c1, c2 and so on.",
    ),
    (
        "^tsg_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the segment_id exactly as VSift printed it: tsg_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^vcd_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the candidate id exactly as VSift printed it: vcd_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$",
        HandoffRule::FormMismatch,
        "Copy expires_at exactly as VSift printed it: YYYY-MM-DDTHH:MM:SSZ.",
    ),
    (
        "^job_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the job id exactly as VSift printed it: job_ and 16 to 64 lowercase letters or digits.",
    ),
    (
        "^(tsg|vcd|evd)_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the identity exactly as VSift printed it: a tsg_, vcd_ or evd_ id.",
    ),
    (
        "^(tsg|evd)_[a-z0-9]{16,64}$",
        HandoffRule::FormMismatch,
        "Copy the identity exactly as VSift printed it: a tsg_ or evd_ id.",
    ),
    (
        "^vsift [a-z]",
        HandoffRule::FormMismatch,
        "The next command is one vsift command line.",
    ),
];

/// The rule and fixed prose a pattern's mismatch reports (or, for a `not`,
/// its match). Every pattern of `handoff.schema.json` has its own entry,
/// which a test checks; a pattern added later falls back to the generic
/// prose until it gets one.
fn pattern_message(source: &str, negated: bool) -> (HandoffRule, &'static str) {
    PATTERN_MESSAGES
        .iter()
        .find(|(pattern, _, _)| *pattern == source)
        .map_or_else(
            || {
                if negated {
                    (
                        HandoffRule::TextForbidden,
                        HandoffRule::TextForbidden.message(),
                    )
                } else {
                    (
                        HandoffRule::FormMismatch,
                        HandoffRule::FormMismatch.message(),
                    )
                }
            },
            |(_, rule, message)| (*rule, *message),
        )
}

/// Whether [`pattern_message`] has an entry of its own for `source`.
pub(crate) fn has_pattern_message(source: &str) -> bool {
    let generic = [
        HandoffRule::TextForbidden.message(),
        HandoffRule::FormMismatch.message(),
    ];
    !generic.contains(&pattern_message(source, false).1)
}

/// The prose of a `not: {const}` that matched.
fn forbidden_value_message(value: &Value) -> &'static str {
    if value == "unsupported" {
        "An observed claim is never unsupported: write a claim you could not check as inferred (or reported) and unsupported."
    } else {
        HandoffRule::ValueForbidden.message()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{CompiledSchema, HandoffSchemaError, translate_pattern};

    #[test]
    fn unimplemented_keywords_and_forms_are_refused() {
        for schema in [
            json!({"type": "object", "patternProperties": {}}),
            json!({"type": "object", "additionalProperties": {"type": "string"}}),
            json!({"if": {"const": 1}, "then": {}, "else": {}}),
            json!({"if": {"const": 1}}),
            json!({"not": {"type": "string"}}),
            json!({"minimum": 1.5}),
            json!({"$ref": "https://example.test/x"}),
            json!({"$ref": "#/$defs/missing"}),
            json!({"type": "decimal"}),
            json!({"format": "date-time"}),
            json!({"$schema": "http://json-schema.org/draft-07/schema#"}),
            json!({"properties": {"a": true}}),
        ] {
            assert!(CompiledSchema::compile(&schema).is_err(), "{schema}");
        }
        assert!(matches!(
            CompiledSchema::compile(&json!({"contains": {}})),
            Err(HandoffSchemaError::UnsupportedKeyword { .. })
        ));
    }

    #[test]
    fn patterns_translate_or_are_refused() {
        assert_eq!(
            translate_pattern(r"^a\sb$").as_deref(),
            Some(r"^a[ \t\n\r\x0B\x0C ﻿  ]b$")
        );
        assert_eq!(
            translate_pattern(r"[\s(=]").as_deref(),
            Some(r"[ \t\n\r\x0B\x0C ﻿  (=]")
        );
        for refused in [r"\d", r"\w", r"\bx", "a.b", "(?=x)", r"\1", "[a"] {
            assert_eq!(translate_pattern(refused), None, "{refused}");
        }
    }
}
