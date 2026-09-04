//! Repository-backed dashboard fixtures and spec builders.

use super::{CliRun, run_powerbi_owned};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const ARCHETYPES: &[&str] = &[
    "sales",
    "catalog-proof",
    "flat-ops",
    "regional-sales",
    "scatter-bubble",
];

/// Paths for one checked-in schema/profile/dashboard/golden fixture family.
#[derive(Clone, Debug)]
pub struct ArchetypeFixture {
    pub name: String,
    pub schema: PathBuf,
    pub profile: PathBuf,
    pub spec: PathBuf,
    pub expected_summary: PathBuf,
}

/// Mutable JSON builder seeded from a checked-in dashboard spec.
#[derive(Clone, Debug)]
pub struct DashboardSpecBuilder {
    value: Value,
}

/// Return every archetype currently backed by schema, profile, spec, and golden files.
pub fn archetype_names() -> &'static [&'static str] {
    ARCHETYPES
}

/// Load a complete checked-in archetype by its stable name.
pub fn load_archetype(name: &str) -> ArchetypeFixture {
    assert!(
        ARCHETYPES.contains(&name),
        "unknown archetype {name:?}; expected one of {ARCHETYPES:?}"
    );
    let (base, golden_base) = if name == "sales" {
        (PathBuf::from("examples"), PathBuf::from("testdata/golden"))
    } else {
        (
            PathBuf::from("examples/archetypes"),
            PathBuf::from("testdata/golden/archetypes"),
        )
    };
    let fixture = ArchetypeFixture {
        name: name.to_string(),
        schema: base.join(format!("{name}.schema.json")),
        profile: base.join(format!("{name}.profile.json")),
        spec: base.join(format!("{name}.dashboard.json")),
        expected_summary: golden_base.join(if name == "sales" {
            "generic-sales.summary.json".to_string()
        } else {
            format!("{name}.summary.json")
        }),
    };
    for path in [
        &fixture.schema,
        &fixture.profile,
        &fixture.spec,
        &fixture.expected_summary,
    ] {
        assert!(
            path.is_file(),
            "archetype input is missing: {}",
            path.display()
        );
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read archetype input {}: {error}", path.display()));
        serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|error| panic!("parse archetype input {}: {error}", path.display()));
    }
    fixture
}

impl ArchetypeFixture {
    /// Invoke `report build` with this fixture into a fresh output directory.
    pub fn build_into(&self, out_dir: &Path) -> CliRun {
        run_powerbi_owned(&[
            "report".into(),
            "build".into(),
            "--schema".into(),
            path_arg(&self.schema),
            "--profile".into(),
            path_arg(&self.profile),
            "--spec".into(),
            path_arg(&self.spec),
            "--out-dir".into(),
            path_arg(out_dir),
            "--json".into(),
        ])
    }

    /// Start a builder that preserves the fixture's declared spec version.
    pub fn spec_builder(&self) -> DashboardSpecBuilder {
        DashboardSpecBuilder::from_path(&self.spec)
    }

    /// Start a dashboard-v2 builder from this real fixture.
    ///
    /// V2 compilation belongs to T2; this helper intentionally only authors
    /// test input and does not imply the current CLI accepts v2 yet.
    pub fn v2_spec_builder(&self) -> DashboardSpecBuilder {
        let mut builder = self.spec_builder();
        builder.value["schema"] = Value::String("powerbi-cli.dashboard.v2".into());
        builder
    }
}

impl DashboardSpecBuilder {
    /// Load a builder from a checked-in dashboard JSON document.
    pub fn from_path(path: &Path) -> Self {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read dashboard spec {}: {error}", path.display()));
        let value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("parse dashboard spec {}: {error}", path.display()));
        Self { value }
    }

    /// Append a visual object to the page with the given spec id.
    pub fn add_visual(mut self, page_id: &str, visual: Value) -> Self {
        page_mut(&mut self.value, page_id)["visuals"]
            .as_array_mut()
            .unwrap_or_else(|| panic!("page {page_id:?} visuals must be an array"))
            .push(visual);
        self
    }

    /// Append a page-scoped filter object, creating `filters` when absent.
    pub fn add_filter(mut self, page_id: &str, filter: Value) -> Self {
        let page = page_mut(&mut self.value, page_id);
        page.as_object_mut()
            .expect("dashboard page must be an object")
            .entry("filters")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .unwrap_or_else(|| panic!("page {page_id:?} filters must be an array"))
            .push(filter);
        self
    }

    /// Replace the interactions array on one page.
    pub fn set_interactions(mut self, page_id: &str, interactions: Value) -> Self {
        assert!(
            interactions.is_array(),
            "page interactions must be a JSON array"
        );
        page_mut(&mut self.value, page_id)["interactions"] = interactions;
        self
    }

    /// Replace the root style section.
    pub fn set_style(mut self, style: Value) -> Self {
        self.value["style"] = style;
        self
    }

    /// Return the authored dashboard JSON value.
    pub fn build(self) -> Value {
        self.value
    }

    /// Write deterministic pretty JSON for a CLI test input.
    pub fn write_to(self, path: &Path) -> Value {
        let value = self.build();
        let mut bytes = serde_json::to_vec_pretty(&value).expect("serialize dashboard spec");
        bytes.push(b'\n');
        fs::write(path, bytes)
            .unwrap_or_else(|error| panic!("write dashboard spec {}: {error}", path.display()));
        value
    }
}

fn page_mut<'a>(spec: &'a mut Value, page_id: &str) -> &'a mut Value {
    spec["pages"]
        .as_array_mut()
        .expect("dashboard pages must be an array")
        .iter_mut()
        .find(|page| page["id"] == page_id)
        .unwrap_or_else(|| panic!("dashboard page id not found: {page_id:?}"))
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("test path is UTF-8").to_string()
}
