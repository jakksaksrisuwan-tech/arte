//! Wire protocol: JSON is the wire format only. Internally everything is a typed
//! enum/struct. The component catalog is fixed here in Rust — no dynamic widgets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One JSONL line = one message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum UiMessage {
    CreateSurface {
        id: String,
        title: String,
        root: UiNode,
    },
    UpdateNode {
        surface_id: String,
        node_id: String,
        patch: UiPatch,
    },
    DeleteNode {
        surface_id: String,
        node_id: String,
    },
}

/// Fixed component catalog. Unknown `type` values are rejected by serde.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum UiNode {
    Panel {
        id: String,
        #[serde(default)]
        title: Option<String>,
        layout: LayoutKind,
        children: Vec<UiNode>,
    },
    Text {
        id: String,
        text: String,
    },
    Metric {
        id: String,
        label: String,
        value: String,
    },
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Log {
        id: String,
        lines: Vec<String>,
    },
    Progress {
        id: String,
        label: String,
        value: f64,
    },
    /// Columns of items. `style` picks how items render: `list` (flat lines, the
    /// intent.map default) or `cards` (each item a small box — the impl.arch look,
    /// columns = layers, items = components). Same data + agent surface either way.
    Board {
        id: String,
        columns: Vec<BoardColumn>,
        #[serde(default)]
        style: BoardStyle,
    },
}

/// How a `Board` renders its items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BoardStyle {
    /// Flat text lines under each header (intent.map / pillars).
    #[default]
    List,
    /// Each item is a small box — components grouped under layer headers (impl.arch).
    Cards,
    /// Registry panel: category sidebar (coarse filter) + filtered list + a
    /// permanent fine-grain filter box (control.spec).
    Panel,
    /// Excel-like grid: a row per item (id · name · status · comment); the
    /// selected row expands to its description + links (validation.rep).
    Sheet,
}

/// One column (lane) of a `Board`: a header plus its stacked item boxes.
/// `key` is a stable, human/agent-facing handle (e.g. "why") for absolute
/// addressing — it survives even as items churn. Columns are the fixed skeleton.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardColumn {
    pub header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub items: Vec<Item>,
}

/// Neutral, domain-agnostic status the *producer* asserts (the tool never
/// computes it — see the design-truth principle). `n/a` = `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    /// Failed. Wire word is `ko` (matches the UI chip); `fail` still parses.
    #[serde(rename = "ko", alias = "fail")]
    Fail,
    Pending,
    /// Failed but accepted by the user — pair with a `note` justification. Does
    /// not count as a failure in coverage roll-up.
    Justified,
}

/// One board item. `serves` lists the intent titles this item realizes (the
/// cross-layer link); `status` is the producer-set verdict. Both ride ON the
/// item — one owner — so they can't desync from it. Serializes as a bare string
/// when plain (cheap wire, readable examples) and as `{text, serves, status}`
/// once linked or stamped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub text: String,
    pub serves: Vec<String>,
    pub status: Option<Status>,
    /// The how/what description — implementer guidance / what this element does,
    /// one line each (validation: what the test does). Shown when expanded.
    pub note: Vec<String>,
    /// Attachment refs (file paths / URLs) — supporting references ("see also"),
    /// the pointer, never the bytes.
    pub attachments: Vec<String>,
    /// Realization locator(s): WHERE this item is realized in the artifact, as
    /// `file#native-unit` (e.g. `auth.rs#hash_password`, `power.kicad_sch#R1`).
    /// The downward dual of `serves` — what realizes THIS. Used by the reconciler
    /// to map a change back to its intent; most precise at control granularity.
    pub at: Vec<String>,
    /// Free-form category tag (producer-defined, e.g. "limits", "authz"). Agnostic.
    pub category: Option<String>,
    /// Producer-supplied stable reference (e.g. a test-case id "TC-001"). SHOWN,
    /// not a link key — links/dedup stay by title. Agnostic; any layer may use it.
    pub id: Option<String>,
    /// A remark on the RESULT (e.g. "flaky under load, accepted"); pairs with
    /// `status`. The how/what description lives in `note` instead.
    pub comment: Option<String>,
    /// Marked as a DERIVED requirement: parentless by design (not an orphan).
    /// The justified-twin on the trace axis; put the rationale in `note`.
    pub derived: bool,
    /// The commit SHA this item was tested/verified against (producer-set, e.g. a
    /// validation case records which build it passed on). Shown short.
    pub sha: Option<String>,
    /// Last-modified — Unix timestamp in SECONDS (not millis), UTC. Stamped by
    /// `touch()` on every edit; used for search/sort. Don't set by hand.
    pub modified: Option<u64>,
}

impl Item {
    pub fn new(text: impl Into<String>) -> Self {
        Item { text: text.into(), ..Default::default() }
    }
    /// True when the item is plain (serializes as a bare string).
    fn is_plain(&self) -> bool {
        self.serves.is_empty()
            && self.status.is_none()
            && self.note.is_empty()
            && self.attachments.is_empty()
            && self.at.is_empty()
            && self.category.is_none()
            && self.id.is_none()
            && self.comment.is_none()
            && !self.derived
            && self.sha.is_none()
            && self.modified.is_none()
    }
    /// Stamp the modified time to now (wall clock, unix secs).
    pub fn touch(&mut self) {
        self.modified = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .ok();
    }
}
impl From<&str> for Item {
    fn from(s: &str) -> Self {
        Item::new(s)
    }
}
impl From<String> for Item {
    fn from(s: String) -> Self {
        Item::new(s)
    }
}

impl Serialize for Item {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.is_plain() {
            return s.serialize_str(&self.text);
        }
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("text", &self.text)?;
        if !self.serves.is_empty() {
            m.serialize_entry("serves", &self.serves)?;
        }
        if let Some(st) = &self.status {
            m.serialize_entry("status", st)?;
        }
        if !self.note.is_empty() {
            m.serialize_entry("note", &self.note)?;
        }
        if !self.attachments.is_empty() {
            m.serialize_entry("attachments", &self.attachments)?;
        }
        if !self.at.is_empty() {
            m.serialize_entry("at", &self.at)?;
        }
        if let Some(c) = &self.category {
            m.serialize_entry("category", c)?;
        }
        if let Some(i) = &self.id {
            m.serialize_entry("id", i)?;
        }
        if let Some(c) = &self.comment {
            m.serialize_entry("comment", c)?;
        }
        if self.derived {
            m.serialize_entry("derived", &true)?;
        }
        if let Some(s) = &self.sha {
            m.serialize_entry("sha", s)?;
        }
        if let Some(t) = &self.modified {
            m.serialize_entry("modified", t)?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for Item {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Obj {
                text: String,
                #[serde(default)]
                serves: Vec<String>,
                #[serde(default)]
                status: Option<Status>,
                #[serde(default)]
                note: Vec<String>,
                #[serde(default)]
                attachments: Vec<String>,
                #[serde(default)]
                at: Vec<String>,
                #[serde(default)]
                category: Option<String>,
                #[serde(default)]
                id: Option<String>,
                #[serde(default)]
                comment: Option<String>,
                #[serde(default)]
                derived: bool,
                #[serde(default)]
                sha: Option<String>,
                #[serde(default)]
                modified: Option<u64>,
            },
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Text(text) => Item::new(text),
            Raw::Obj { text, serves, status, note, attachments, at, category, id, comment, derived, sha, modified } => {
                Item { text, serves, status, note, attachments, at, category, id, comment, derived, sha, modified }
            }
        })
    }
}

// JsonSchema: an item is a string OR { text, serves, status } — keep the exported
// schema honest so generated client types accept both.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct ItemObj {
    text: String,
    #[serde(default)]
    serves: Vec<String>,
    #[serde(default)]
    status: Option<Status>,
    #[serde(default)]
    note: Vec<String>,
    #[serde(default)]
    attachments: Vec<String>,
    #[serde(default)]
    at: Vec<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    derived: bool,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    modified: Option<u64>,
}
impl JsonSchema for Item {
    fn schema_name() -> String {
        "Item".into()
    }
    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Schema, SchemaObject, SubschemaValidation};
        let as_string = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        });
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![as_string, gen.subschema_for::<ItemObj>()]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

impl UiNode {
    pub fn id(&self) -> &str {
        match self {
            UiNode::Panel { id, .. }
            | UiNode::Text { id, .. }
            | UiNode::Metric { id, .. }
            | UiNode::Table { id, .. }
            | UiNode::Log { id, .. }
            | UiNode::Progress { id, .. }
            | UiNode::Board { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum LayoutKind {
    Vertical,
    Horizontal,
}

/// Partial update applied to an existing node by id. Only set fields change.
/// Each field maps to whichever node kinds carry it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiPatch {
    /// Panel title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Text node body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Metric label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Metric value (string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Progress value, 0.0..=1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// Log lines (replaces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// Table rows (replaces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<String>>>,
    /// Board columns (replaces) — how you add/remove board items live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<Vec<BoardColumn>>,
}


#[cfg(test)]
mod tests {
    use super::*;

    // Guards against serde drift: a new Item field that isn't wired into both
    // Serialize and Deserialize will fail this round-trip (and the schema review).
    #[test]
    fn item_roundtrips_every_field() {
        let it = Item {
            text: "t".into(),
            serves: vec!["a".into()],
            status: Some(Status::Fail),
            note: vec!["n".into()],
            attachments: vec!["@x".into()],
            at: vec!["src/x.rs#f".into()],
            category: Some("cat".into()),
            id: Some("tsts_abc_001".into()),
            comment: Some("c".into()),
            derived: true,
            sha: Some("deadbeef".into()),
            modified: Some(123),
        };
        let json = serde_json::to_string(&it).unwrap();
        let back: Item = serde_json::from_str(&json).unwrap();
        assert_eq!(it, back, "every field survives serialize → deserialize: {json}");
        assert!(json.contains("\"ko\""), "Fail serializes as the UI word ko: {json}");
        let aliased = json.replace("\"ko\"", "\"fail\"");
        assert_eq!(serde_json::from_str::<Item>(&aliased).unwrap().status, Some(Status::Fail), "fail still parses as ko");
    }
}
