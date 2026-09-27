//! The camelCase naming check for property and parameter names.

use std::fmt;

use serde_json::{Map, Value};

/// Keywords whose values are data or product-owned identifiers, never wire field names.
const DATA_KEYWORDS: [&str; 6] = [
    "const",
    "default",
    "discriminator",
    "enum",
    "example",
    "examples",
];
/// Parameter locations the naming convention covers. Header names follow HTTP conventions.
const CHECKED_PARAMETER_LOCATIONS: [&str; 2] = ["path", "query"];

/// The kind of name a [`NamingViolation`] refers to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameKind {
    /// A JSON property name in a schema's `properties` map.
    Property,
    /// A path or query parameter name.
    Parameter,
}

impl fmt::Display for NameKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Property => "property",
            Self::Parameter => "parameter",
        })
    }
}

/// A property or parameter name that is not camelCase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamingViolation {
    /// JSON pointer to the property entry or the parameter object.
    pub pointer: String,
    /// The offending name.
    pub name: String,
    /// Whether the name is a property or a parameter.
    pub kind: NameKind,
}

impl fmt::Display for NamingViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {:?} at {}",
            self.kind, self.name, self.pointer
        )
    }
}

/// Returns `true` for a lower camelCase name: an ASCII lowercase letter followed by ASCII letters
/// and digits.
#[must_use]
pub fn is_camel_case(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_alphanumeric())
}

/// Finds every property and parameter name in a serialized OpenAPI document that is not camelCase.
///
/// The walk covers schema `properties` keys, including multipart form fields, and path and query
/// parameter names. It never inspects enum or const values, examples, defaults, discriminator
/// mappings, or `x-` extensions, and `additionalProperties` map keys never appear in a document.
/// Names listed in `exemptions`, such as OAuth 2.0 `access_token`, are accepted wherever they occur.
#[must_use]
pub fn find_naming_violations(document: &Value, exemptions: &[&str]) -> Vec<NamingViolation> {
    let mut walker = Walker {
        exemptions,
        violations: Vec::new(),
    };
    walker.walk(document, &mut String::new());
    walker.violations
}

struct Walker<'a> {
    exemptions: &'a [&'a str],
    violations: Vec<NamingViolation>,
}

impl Walker<'_> {
    fn walk(&mut self, value: &Value, pointer: &mut String) {
        match value {
            Value::Object(object) => self.walk_object(object, pointer),
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    with_segment(pointer, &index.to_string(), |pointer| {
                        self.walk(item, pointer)
                    });
                }
            }
            _ => {}
        }
    }

    fn walk_object(&mut self, object: &Map<String, Value>, pointer: &mut String) {
        self.check_parameter(object, pointer);
        for (key, child) in object {
            if is_skipped_keyword(key) {
                continue;
            }
            with_segment(pointer, key, |pointer| match (key.as_str(), child) {
                ("properties", Value::Object(properties)) => {
                    self.walk_properties(properties, pointer)
                }
                _ => self.walk(child, pointer),
            });
        }
    }

    fn walk_properties(&mut self, properties: &Map<String, Value>, pointer: &mut String) {
        for (name, schema) in properties {
            with_segment(pointer, name, |pointer| {
                self.check_name(name, pointer, NameKind::Property);
                self.walk(schema, pointer);
            });
        }
    }

    fn check_parameter(&mut self, object: &Map<String, Value>, pointer: &str) {
        let (Some(Value::String(location)), Some(Value::String(name))) =
            (object.get("in"), object.get("name"))
        else {
            return;
        };
        if CHECKED_PARAMETER_LOCATIONS.contains(&location.as_str()) {
            self.check_name(name, pointer, NameKind::Parameter);
        }
    }

    fn check_name(&mut self, name: &str, pointer: &str, kind: NameKind) {
        if is_camel_case(name) || self.exemptions.contains(&name) {
            return;
        }
        self.violations.push(NamingViolation {
            pointer: pointer.to_owned(),
            name: name.to_owned(),
            kind,
        });
    }
}

fn is_skipped_keyword(key: &str) -> bool {
    key.starts_with("x-") || DATA_KEYWORDS.contains(&key)
}

fn with_segment(pointer: &mut String, segment: &str, visit: impl FnOnce(&mut String)) {
    let length = pointer.len();
    pointer.push('/');
    pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    visit(pointer);
    pointer.truncate(length);
}
