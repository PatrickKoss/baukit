use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    rc::Rc,
};

use baukit_cli::{Manifest, check::rule_names, cli::command};
use serde::{
    Deserialize,
    de::{
        self, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
        Visitor, value::Error,
    },
};

const CLI_GUIDE: &str = "docs/guide/cli.md";
const MANIFEST_GUIDE: &str = "docs/guide/manifest.md";
const MAX_TRACE_ROUNDS: usize = 16;

fn repository_page(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{relative} is unreadable: {error}"))
}

fn assert_page_documents(relative: &str, missing: impl Fn(&str) -> Vec<String>) {
    let gaps = missing(&repository_page(relative));
    assert!(gaps.is_empty(), "{relative} does not mention: {gaps:?}");
}

fn cli_terms() -> Vec<String> {
    fn walk(command: &clap::Command, prefix: &str, terms: &mut Vec<String>) {
        for argument in command.get_arguments() {
            if let Some(long) = argument.get_long() {
                terms.push(format!("--{long}"));
            }
        }
        for subcommand in command.get_subcommands() {
            let path = format!("{prefix} {}", subcommand.get_name());
            terms.push(path.clone());
            walk(subcommand, &path, terms);
        }
    }
    let mut terms = Vec::new();
    walk(&command(), "baukit", &mut terms);
    terms.extend(rule_names().into_iter().map(|rule| format!("`{rule}`")));
    terms.sort();
    terms.dedup();
    terms
}

fn missing_cli_terms(page: &str) -> Vec<String> {
    cli_terms()
        .into_iter()
        .filter(|term| !contains_term(page, term))
        .collect()
}

fn contains_term(page: &str, term: &str) -> bool {
    page.match_indices(term).any(|(start, _)| {
        let next = page[start + term.len()..].chars().next();
        !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

fn names_section(page: &str, section: &str) -> bool {
    [
        format!("[{section}]"),
        format!("`{section}`"),
        format!("[{section}."),
    ]
    .iter()
    .any(|form| page.contains(form.as_str()))
}

fn names_key_of_parent(page: &str, section: &str) -> bool {
    section.rsplit_once('.').is_some_and(|(parent, key)| {
        names_key_of_parent_or_section(page, parent) && page.contains(&format!("| `{key}` |"))
    })
}

fn names_key_of_parent_or_section(page: &str, section: &str) -> bool {
    names_section(page, section) || names_key_of_parent(page, section)
}

fn missing_sections(page: &str, sections: &BTreeSet<String>) -> Vec<String> {
    sections
        .iter()
        .filter(|section| !names_key_of_parent_or_section(page, section))
        .cloned()
        .collect()
}

#[derive(Default)]
struct Trace {
    tables: BTreeSet<String>,
    failed: BTreeMap<String, usize>,
    round: usize,
}

#[derive(Clone)]
struct Tracer {
    path: String,
    trace: Rc<RefCell<Trace>>,
}

impl Tracer {
    fn child(&self, key: &str) -> Self {
        let path = if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        };
        Self {
            path,
            trace: Rc::clone(&self.trace),
        }
    }

    fn table(&self) {
        if !self.path.is_empty() {
            self.trace.borrow_mut().tables.insert(self.path.clone());
        }
    }

    fn fields(&self, keys: &[&'static str]) -> Fields {
        let trace = self.trace.borrow();
        let keys = keys
            .iter()
            .copied()
            .filter(|key| !trace.failed.contains_key(&self.child(key).path))
            .collect::<Vec<_>>();
        Fields {
            tracer: self.clone(),
            keys: keys.into_iter(),
            current: "",
        }
    }
}

struct Fields {
    tracer: Tracer,
    keys: std::vec::IntoIter<&'static str>,
    current: &'static str,
}

impl<'de> MapAccess<'de> for Fields {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        let Some(key) = self.keys.next() else {
            return Ok(None);
        };
        self.current = key;
        seed.deserialize(IntoDeserializer::<Error>::into_deserializer(key))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let child = self.tracer.child(self.current);
        seed.deserialize(child.clone()).inspect_err(|_| {
            let mut trace = child.trace.borrow_mut();
            let nested = format!("{}.", child.path);
            let round = trace.round;
            let inner_failed = trace
                .failed
                .iter()
                .any(|(path, failed_in)| *failed_in == round && path.starts_with(&nested));
            if !inner_failed {
                trace.failed.insert(child.path.clone(), round);
            }
        })
    }
}

struct OneElement(Option<Tracer>);

impl<'de> SeqAccess<'de> for OneElement {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        self.0
            .take()
            .map(|tracer| seed.deserialize(tracer))
            .transpose()
    }
}

struct FirstVariant {
    variant: &'static str,
    tracer: Tracer,
}

impl<'de> EnumAccess<'de> for FirstVariant {
    type Error = Error;
    type Variant = Tracer;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Tracer), Error> {
        let value = seed.deserialize(IntoDeserializer::<Error>::into_deserializer(self.variant))?;
        Ok((value, self.tracer))
    }
}

impl<'de> VariantAccess<'de> for Tracer {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_seq(OneElement(Some(self)))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.table();
        visitor.visit_map(self.fields(fields))
    }
}

impl<'de> de::Deserializer<'de> for Tracer {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_map(visitor)
    }
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_bool(false)
    }
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(1)
    }
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_f64(1.0)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_f64(1.0)
    }
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_char('a')
    }
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_str("a")
    }
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_str("a")
    }
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_bytes(b"a")
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_bytes(b"a")
    }
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_some(self)
    }
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_seq(OneElement(Some(self)))
    }
    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_seq(OneElement(Some(self)))
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_seq(OneElement(Some(self)))
    }
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.table();
        visitor.visit_map(self.fields(&[]))
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.table();
        visitor.visit_map(self.fields(fields))
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let variant = variants.first().copied().unwrap_or_default();
        visitor.visit_enum(FirstVariant {
            variant,
            tracer: self,
        })
    }
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_str("a")
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }
}

/// Every table a `T` reads, found by deserializing it from a tracer. A field that fails to
/// deserialize is left out of the next round, so the fields after it are still traced.
fn traced_tables<T: for<'de> Deserialize<'de>>() -> BTreeSet<String> {
    let trace = Rc::new(RefCell::new(Trace::default()));
    for round in 0..MAX_TRACE_ROUNDS {
        trace.borrow_mut().round = round;
        let failed_before = trace.borrow().failed.len();
        let tracer = Tracer {
            path: String::new(),
            trace: Rc::clone(&trace),
        };
        if T::deserialize(tracer).is_ok() || trace.borrow().failed.len() == failed_before {
            break;
        }
    }
    std::mem::take(&mut trace.borrow_mut().tables)
}

fn manifest_sections() -> BTreeSet<String> {
    traced_tables::<Manifest>()
}

#[test]
fn cli_guide_names_every_subcommand_flag_and_check_rule() {
    assert_page_documents(CLI_GUIDE, missing_cli_terms);
}

#[test]
fn manifest_guide_names_every_baukit_toml_section() {
    let sections = manifest_sections();
    assert_page_documents(MANIFEST_GUIDE, |page| missing_sections(page, &sections));
}

#[test]
fn cli_terms_cover_nested_subcommands_flags_and_rules() {
    let terms = cli_terms();
    for term in [
        "baukit new",
        "baukit generate openapi-client",
        "--baukit-path",
        "--check",
        "`security`",
    ] {
        assert!(
            terms.iter().any(|known| known == term),
            "{term} in {terms:?}"
        );
    }
}

#[test]
fn missing_cli_terms_reports_a_planted_gap() {
    let page = cli_terms()
        .into_iter()
        .filter(|term| term != "--into-existing")
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(missing_cli_terms(&page), vec!["--into-existing".to_owned()]);
}

#[test]
fn a_longer_flag_does_not_satisfy_its_prefix() {
    assert!(!contains_term("--forced", "--force"));
    assert!(contains_term("use `--force`.", "--force"));
}

#[test]
fn manifest_sections_follow_the_serde_structs() {
    let sections = manifest_sections();
    for section in [
        "app",
        "quality",
        "quality.ci",
        "gate",
        "gate.worker-caps",
        "backend.image",
        "postgres",
        "openapi",
    ] {
        assert!(sections.contains(section), "{section} in {sections:?}");
    }
    assert!(!sections.contains("schema_version"), "{sections:?}");
}

#[test]
fn missing_sections_accepts_headings_and_code_spans() {
    let sections = ["app", "quality.ci", "gitops"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        missing_sections("## [app]\nSet `quality.ci` here.", &sections),
        vec!["gitops".to_owned()]
    );
}

#[test]
fn missing_sections_accepts_subtables_and_keys_of_a_named_parent() {
    let sections = [
        "dependencies",
        "backend.image.downloads",
        "backend.image.pre_build",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let page = "## `[dependencies.baukit]`\n## `[backend.image]`\n| `downloads` | `[]` |\n";
    assert_eq!(
        missing_sections(page, &sections),
        vec!["backend.image.pre_build".to_owned()]
    );
}
