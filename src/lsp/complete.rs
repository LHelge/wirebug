//! Context-aware completion: an owned snapshot of the resolved registry
//! plus a token-stack scan of the live buffer.
//!
//! The index is rebuilt from [`Resolved`] after every check and kept as
//! *last-good* when the current buffer is too broken to load — mid-edit
//! buffers usually are, and the instance/port sets are stable across a
//! keystroke. The buffer at the cursor, by contrast, is always the live
//! overlay text: it is lexed (not parsed) and a block stack derives the
//! completion context, so completion works in code the parser rejects.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lsp_types::{CompletionItem, CompletionItemKind};

use crate::dsl::ir::ColorName;
use crate::dsl::lex::{Token, lex, significant};
use crate::dsl::project::Project;
use crate::dsl::resolve::Resolved;
use crate::dsl::span::FileId;

const SIDES: [&str; 4] = ["north", "east", "south", "west"];
const VIEW_KINDS: [&str; 3] = ["schematic", "harness", "pinout"];
const NUMBERING_MODES: [&str; 4] = ["row_major", "odd_even", "clockwise", "counter_clockwise"];

/// Key of a merged component in [`CompletionIndex::components`]. Stable
/// only within one index build.
type ComponentKey = usize;

#[derive(Default)]
pub(crate) struct CompletionIndex {
    /// Per-file scope, keyed by the loader's canonical path.
    files: HashMap<PathBuf, FileScope>,
    components: HashMap<ComponentKey, ComponentEntry>,
    /// First key unused by this index; [`absorb`](Self::absorb) shifts an
    /// incoming index past it so keys never collide across projects.
    next_key: ComponentKey,
}

#[derive(Default)]
struct FileScope {
    /// Component type names usable in this file (own top-level + imports).
    component_types: Vec<String>,
    /// Connector type names usable in this file (own + imports).
    connector_types: Vec<String>,
    /// The file's top-level components by name (canonical across `extend`).
    top_level: HashMap<String, ComponentKey>,
    /// Connector types *defined* in this file (no imports) — what another
    /// file could `use` from it.
    own_connector_types: Vec<String>,
    /// The file's lone top-level component — the subject of its views.
    subject: Option<ComponentKey>,
}

#[derive(Default)]
struct ComponentEntry {
    instances: Vec<InstanceEntry>,
    ports: Vec<PortEntry>,
    /// Connector designators: named inline `connector` groups and typed
    /// connector instances (what harness/pinout includes refer to).
    connectors: Vec<String>,
    /// Nested component types, visible only inside this component.
    nested: HashMap<String, ComponentKey>,
}

struct InstanceEntry {
    name: String,
    type_name: String,
    type_key: Option<ComponentKey>,
}

struct PortEntry {
    name: String,
    label: String,
    public: bool,
}

/// Build the snapshot for one resolved project.
pub(crate) fn build_index(project: &Project, resolved: &Resolved) -> CompletionIndex {
    let mut components: HashMap<ComponentKey, ComponentEntry> = HashMap::new();

    // Fragments of a merged component all contribute to the canonical key,
    // so a wire in main.wb completes against instances declared in hv.wb.
    for (id, def) in resolved.defs.iter().enumerate() {
        let entry = components.entry(resolved.groups.canonical(id)).or_default();
        for (name, facts) in &def.ports {
            entry.ports.push(PortEntry {
                name: name.to_string(),
                label: facts.label.to_string(),
                public: facts.visibility == crate::dsl::ast::Visibility::Public,
            });
            if let Some(designator) = facts.connector.as_ref().map(|c| c.name)
                && !entry.connectors.iter().any(|c| c == designator)
            {
                entry.connectors.push(designator.to_string());
            }
        }
        for (name, inst) in &def.instances {
            entry.instances.push(InstanceEntry {
                name: name.to_string(),
                type_name: inst.ast.type_name.node.as_str().to_string(),
                type_key: inst.type_id.map(|t| resolved.groups.canonical(t)),
            });
        }
        for name in def.connectors.keys() {
            entry.connectors.push(name.to_string());
        }
        for &nested in &def.nested {
            entry.nested.insert(
                resolved.defs[nested].name.to_string(),
                resolved.groups.canonical(nested),
            );
        }
    }

    let connector_type_names: Vec<&str> =
        resolved.connector_types.iter().map(|ct| ct.name).collect();

    let mut files = HashMap::new();
    for (fi, file) in project.files.iter().enumerate() {
        let mut scope = FileScope::default();
        let mut top_level_here = Vec::new();
        for (id, def) in resolved.defs.iter().enumerate() {
            if def.file == FileId(fi) && def.parent.is_none() {
                let key = resolved.groups.canonical(id);
                scope.component_types.push(def.name.to_string());
                scope.top_level.insert(def.name.to_string(), key);
                top_level_here.push(key);
            }
        }
        if let [only] = top_level_here.as_slice() {
            scope.subject = Some(*only);
        }
        for ct in &resolved.connector_types {
            if ct.file == FileId(fi) {
                scope.connector_types.push(ct.name.to_string());
                scope.own_connector_types.push(ct.name.to_string());
            }
        }
        // An import is a component or a connector type; sort each name into
        // the bucket its definition says it belongs to.
        for import in &file.ast.uses {
            let name = import.name.node.as_str();
            if connector_type_names.contains(&name) {
                scope.connector_types.push(name.to_string());
            } else {
                scope.component_types.push(name.to_string());
            }
        }
        files.insert(file.path.clone(), scope);
    }

    let next_key = resolved.defs.len();
    CompletionIndex {
        files,
        components,
        next_key,
    }
}

impl CompletionIndex {
    /// Merge `other` (another project's index) in, shifting its component
    /// keys so they cannot collide with ours.
    pub(crate) fn absorb(&mut self, other: CompletionIndex) {
        let base = self.next_key;
        for (key, mut entry) in other.components {
            for inst in &mut entry.instances {
                inst.type_key = inst.type_key.map(|k| k + base);
            }
            for key in entry.nested.values_mut() {
                *key += base;
            }
            self.components.insert(key + base, entry);
        }
        for (path, mut scope) in other.files {
            for key in scope.top_level.values_mut() {
                *key += base;
            }
            scope.subject = scope.subject.map(|k| k + base);
            self.files.insert(path, scope);
        }
        self.next_key += other.next_key;
    }

    /// Register an open file no project loads (rust-analyzer's "unlinked
    /// file"): its top-level definitions become auto-import and `extend`
    /// candidates before the first `use` reaches the file — exactly the
    /// window in which the author is wiring a fresh fragment into the
    /// project. Only names are indexed; the file's contents stay unchecked.
    pub(crate) fn add_unlinked(&mut self, path: PathBuf, ast: &crate::dsl::ast::File) {
        let mut scope = FileScope::default();
        for item in &ast.items {
            match item {
                crate::dsl::ast::Item::Definition(def) => {
                    let name = def.name.node.as_str().to_string();
                    scope.component_types.push(name.clone());
                    scope.top_level.insert(name, self.next_key);
                    self.next_key += 1;
                }
                crate::dsl::ast::Item::ConnectorType(ct) => {
                    let name = ct.name.node.as_str().to_string();
                    scope.connector_types.push(name.clone());
                    scope.own_connector_types.push(name);
                }
                _ => {}
            }
        }
        for import in &ast.uses {
            scope
                .component_types
                .push(import.name.node.as_str().to_string());
        }
        self.files.insert(path, scope);
    }

    /// Replace this index with `new`, except that files `new` *lost* keep
    /// their previous scope and the component data it references. A buffer
    /// mid-edit routinely fails to parse, which drops its file from the
    /// load — without this, one broken keystroke kills completion in that
    /// file (and a broken entry file would kill the whole project's).
    pub(crate) fn update_with(&mut self, new: CompletionIndex) {
        self.files.retain(|path, _| !new.files.contains_key(path));
        if self.files.is_empty() {
            *self = new;
            return;
        }
        self.retain_reachable();
        self.absorb(new);
    }

    /// Drop component entries unreachable from the remaining file scopes,
    /// so retained leftovers can't accumulate across rebuilds.
    fn retain_reachable(&mut self) {
        let mut queue: Vec<ComponentKey> = self
            .files
            .values()
            .flat_map(|scope| scope.top_level.values().copied().chain(scope.subject))
            .collect();
        let mut keep = HashSet::new();
        while let Some(key) = queue.pop() {
            if !keep.insert(key) {
                continue;
            }
            if let Some(entry) = self.components.get(&key) {
                queue.extend(entry.nested.values().copied());
                queue.extend(entry.instances.iter().filter_map(|i| i.type_key));
            }
        }
        self.components.retain(|key, _| keep.contains(key));
    }

    /// Complete at byte `offset` in the live `text` of the document at
    /// `path` (the loader-canonical path the index is keyed by).
    pub(crate) fn complete(&self, path: &Path, text: &str, offset: usize) -> Vec<CompletionItem> {
        let Some(scope) = self.files.get(path) else {
            return Vec::new();
        };
        let cursor = scan(text, offset);
        self.items(path, scope, &cursor)
    }

    fn component(&self, key: ComponentKey) -> Option<&ComponentEntry> {
        self.components.get(&key)
    }

    /// Resolve the cursor's innermost component by walking the block stack
    /// from the file's top level down through nested definitions.
    fn current_component(&self, scope: &FileScope, cursor: &Cursor) -> Option<ComponentKey> {
        let mut key: Option<ComponentKey> = None;
        for block in &cursor.blocks {
            if let Block::Component { name } = block {
                key = match key.and_then(|k| self.component(k)) {
                    Some(entry) => entry.nested.get(name).copied(),
                    None => scope.top_level.get(name).copied(),
                };
            }
        }
        key
    }

    /// Component types usable at the cursor: the file scope plus the
    /// nested types of every enclosing component.
    fn visible_types(&self, scope: &FileScope, cursor: &Cursor) -> Vec<CompletionItem> {
        let mut items: Vec<CompletionItem> = scope
            .component_types
            .iter()
            .map(|name| item(name, CompletionItemKind::CLASS, "component"))
            .collect();
        let mut key: Option<ComponentKey> = None;
        for block in &cursor.blocks {
            if let Block::Component { name } = block {
                key = match key.and_then(|k| self.component(k)) {
                    Some(entry) => entry.nested.get(name).copied(),
                    None => scope.top_level.get(name).copied(),
                };
                if let Some(entry) = key.and_then(|k| self.component(k)) {
                    items.extend(
                        entry
                            .nested
                            .keys()
                            .map(|name| item(name, CompletionItemKind::CLASS, "nested component")),
                    );
                }
            }
        }
        items
    }

    fn ports(&self, key: ComponentKey, public_only: bool) -> Vec<CompletionItem> {
        self.component(key)
            .map(|entry| {
                entry
                    .ports
                    .iter()
                    .filter(|p| !public_only || p.public)
                    .map(|p| item(&p.name, CompletionItemKind::FIELD, &p.label))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn instances(&self, key: ComponentKey) -> Vec<CompletionItem> {
        self.component(key)
            .map(|entry| {
                entry
                    .instances
                    .iter()
                    .map(|i| item(&i.name, CompletionItemKind::VARIABLE, &i.type_name))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn connectors(&self, key: ComponentKey) -> Vec<CompletionItem> {
        self.component(key)
            .map(|entry| {
                entry
                    .connectors
                    .iter()
                    .map(|c| item(c, CompletionItemKind::STRUCT, "connector"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The pub ports of a named instance's type within `component`.
    fn instance_ports(&self, component: ComponentKey, instance: &str) -> Vec<CompletionItem> {
        self.component(component)
            .and_then(|entry| entry.instances.iter().find(|i| i.name == instance))
            .and_then(|i| i.type_key)
            .map(|key| self.ports(key, true))
            .unwrap_or_default()
    }

    fn items(&self, doc: &Path, scope: &FileScope, cursor: &Cursor) -> Vec<CompletionItem> {
        // `numbering: ` takes its closed mode set wherever it appears.
        if cursor.after(&[Token::Numbering, Token::Colon]) {
            return constants(&NUMBERING_MODES);
        }
        if cursor.after(&[Token::View]) {
            return constants(&VIEW_KINDS);
        }
        // `wire <color>` and the tracer of a two-tone `<base>/<tracer>`
        // both take the closed IEC 60757 color set.
        if let [.., Token::Wire] | [.., Token::Wire, Token::Ident(_), Token::Slash] = cursor.tail()
        {
            return colors();
        }

        let component = self.current_component(scope, cursor);

        // Wire endpoint lists complete the same way in a component body
        // and inside a cable.
        if cursor.in_list {
            let Some(component) = component else {
                return Vec::new();
            };
            if let [.., Token::Ident(instance), Token::Dot] = cursor.tail() {
                return self.instance_ports(component, instance);
            }
            let mut items = self.instances(component);
            items.extend(self.ports(component, false));
            return items;
        }

        match cursor.blocks.last() {
            None => {
                if let [.., Token::Use, Token::Ident(_)] = cursor.tail() {
                    return keywords(&["from"]);
                }
                if let [.., Token::Use] = cursor.tail() {
                    return self.auto_imports(doc);
                }
                if let [.., Token::Extend] = cursor.tail() {
                    return self.extend_targets();
                }
                if cursor.at_statement_start() {
                    return keywords(&["use", "component", "extend", "connector_type", "view"]);
                }
                Vec::new()
            }
            Some(Block::Component { .. }) => match cursor.tail() {
                [.., Token::Connector, Token::Ident(_), Token::Colon] => scope
                    .connector_types
                    .iter()
                    .map(|name| item(name, CompletionItemKind::INTERFACE, "connector type"))
                    .collect(),
                [.., Token::Ident(_), Token::Colon] => self.visible_types(scope, cursor),
                [.., Token::Pub] => keywords(&["port"]),
                _ if cursor.at_statement_start() => {
                    keywords(&["pub", "port", "wire", "cable", "connector", "component"])
                }
                _ => Vec::new(),
            },
            Some(Block::Cable) => {
                if cursor.at_statement_start() {
                    let mut items = keywords(&["wire", "twisted"]);
                    items.push(property("type", "cable property"));
                    items.push(property("length", "cable property"));
                    return items;
                }
                Vec::new()
            }
            Some(Block::Twisted) => {
                if cursor.at_statement_start() {
                    return keywords(&["wire"]);
                }
                Vec::new()
            }
            Some(Block::Connector) => match cursor.tail() {
                [.., Token::Pub] => keywords(&["port"]),
                _ if cursor.at_statement_start() => keywords(&["pub", "port"]),
                _ => Vec::new(),
            },
            Some(Block::View { kind }) => match cursor.tail() {
                [.., Token::Include, Token::Ident(instance), Token::Dot] => scope
                    .subject
                    .map(|s| self.instance_connectors(s, instance))
                    .unwrap_or_default(),
                [.., Token::Include] => {
                    let Some(subject) = scope.subject else {
                        return Vec::new();
                    };
                    match kind.as_deref() {
                        Some("pinout") => self.connectors(subject),
                        _ => self.instances(subject),
                    }
                }
                _ if cursor.at_statement_start() => {
                    keywords(&["grid", "include", "text", "enclosure"])
                }
                _ => Vec::new(),
            },
            Some(Block::Ports { instance }) => {
                if matches!(cursor.tail(), [.., Token::Colon] | [.., Token::Comma]) {
                    let (Some(subject), Some(instance)) = (scope.subject, instance) else {
                        return Vec::new();
                    };
                    return self.instance_ports(subject, instance);
                }
                if cursor.at_statement_start() {
                    return SIDES
                        .iter()
                        .map(|side| CompletionItem {
                            insert_text: Some(format!("{side}: ")),
                            ..item(side, CompletionItemKind::CONSTANT, "side")
                        })
                        .collect();
                }
                Vec::new()
            }
            Some(Block::Enclosure) => {
                if cursor.in_at_parens {
                    return constants(&SIDES);
                }
                if cursor.at_statement_start() {
                    return scope
                        .subject
                        .map(|s| self.ports(s, true))
                        .unwrap_or_default();
                }
                Vec::new()
            }
            Some(Block::ConnectorType) => match cursor.tail() {
                [.., Token::Layout] => keywords(&["grid", "face"]),
                _ if cursor.at_statement_start() => {
                    let mut items = keywords(&["layout"]);
                    items.push(property("part", "connector property"));
                    items
                }
                _ => Vec::new(),
            },
            Some(Block::LayoutGrid) => {
                if cursor.at_statement_start() {
                    return ["rows", "cols", "numbering"]
                        .iter()
                        .map(|key| property(key, "grid layout"))
                        .collect();
                }
                Vec::new()
            }
            Some(Block::LayoutFace) => match cursor.tail() {
                [.., Token::Size] => constants(&["large"]),
                [.., Token::RParen] => keywords(&["size"]),
                _ if cursor.at_statement_start() => keywords(&["cavity"]),
                _ => Vec::new(),
            },
            Some(Block::Other) => Vec::new(),
        }
    }

    /// Every top-level component name in the project — what an `extend`
    /// may name. Deduplicated (a merged component is declared in several
    /// files) and sorted for a stable order.
    fn extend_targets(&self) -> Vec<CompletionItem> {
        let names: std::collections::BTreeSet<&str> = self
            .files
            .values()
            .flat_map(|scope| scope.top_level.keys().map(String::as_str))
            .collect();
        names
            .into_iter()
            .map(|name| item(name, CompletionItemKind::CLASS, "component"))
            .collect()
    }

    /// Auto-import items for `use ‸`: every component and connector type
    /// defined in *another* file, inserting the full
    /// `<Name> from "<relative path>";` remainder of the statement.
    fn auto_imports(&self, doc: &Path) -> Vec<CompletionItem> {
        let Some(doc_dir) = doc.parent() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for (path, scope) in &self.files {
            if path == doc {
                continue;
            }
            let rel = relative_path(doc_dir, path);
            let import = |name: &str, kind, detail: &str| CompletionItem {
                insert_text: Some(format!("{name} from \"{rel}\";")),
                ..CompletionItem {
                    detail: Some(format!("{detail} · {rel}")),
                    ..item(name, kind, detail)
                }
            };
            for name in scope.top_level.keys() {
                items.push(import(name, CompletionItemKind::CLASS, "component"));
            }
            for name in &scope.own_connector_types {
                items.push(import(
                    name,
                    CompletionItemKind::INTERFACE,
                    "connector type",
                ));
            }
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items
    }

    /// The connector designators of a named instance's type — what a
    /// harness `include inst.<connector>` refers to.
    fn instance_connectors(&self, subject: ComponentKey, instance: &str) -> Vec<CompletionItem> {
        self.component(subject)
            .and_then(|entry| entry.instances.iter().find(|i| i.name == instance))
            .and_then(|i| i.type_key)
            .map(|key| self.connectors(key))
            .unwrap_or_default()
    }
}

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        ..CompletionItem::default()
    }
}

fn keywords(words: &[&str]) -> Vec<CompletionItem> {
    words
        .iter()
        .map(|w| item(w, CompletionItemKind::KEYWORD, "keyword"))
        .collect()
}

fn constants(values: &[&str]) -> Vec<CompletionItem> {
    values
        .iter()
        .map(|v| item(v, CompletionItemKind::CONSTANT, ""))
        .collect()
}

/// The IEC 60757 colors, canonical name labelled with its letter code.
fn colors() -> Vec<CompletionItem> {
    ColorName::STANDARD
        .iter()
        .map(|c| item(c.css(), CompletionItemKind::COLOR, c.code()))
        .collect()
}

fn property(key: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        insert_text: Some(format!("{key}: ")),
        ..item(key, CompletionItemKind::PROPERTY, detail)
    }
}

/// `to` relative to `from_dir`, with forward slashes — the form a `use`
/// path is written in. Both paths must be absolute (they come from the
/// loader's canonicalization).
fn relative_path(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// One nesting level at the cursor, tagged by its governing keyword.
enum Block {
    Component {
        name: String,
    },
    ConnectorType,
    Connector,
    Cable,
    Twisted,
    View {
        kind: Option<String>,
    },
    Ports {
        instance: Option<String>,
    },
    Enclosure,
    /// `layout grid { }` inside a connector_type.
    LayoutGrid,
    /// `layout face { }` inside a connector_type.
    LayoutFace,
    Other,
}

/// Everything the item rules need to know about the cursor: the block
/// stack, list/paren flags, and the significant tokens before it.
struct Cursor {
    blocks: Vec<Block>,
    in_list: bool,
    in_at_parens: bool,
    tokens: Vec<Token>,
}

impl Cursor {
    /// The last few tokens before the cursor (for slice-pattern lookback).
    fn tail(&self) -> &[Token] {
        let n = self.tokens.len();
        &self.tokens[n.saturating_sub(4)..]
    }

    fn after(&self, suffix: &[Token]) -> bool {
        self.tokens.ends_with(suffix)
    }

    fn at_statement_start(&self) -> bool {
        matches!(
            self.tokens.last(),
            None | Some(Token::LBrace | Token::RBrace | Token::Semicolon)
        )
    }
}

/// Lex the live buffer and fold the tokens before `offset` into a
/// [`Cursor`]. A lexically broken buffer degrades to its valid prefix —
/// the cursor usually sits past the text that still lexes.
fn scan(text: &str, offset: usize) -> Cursor {
    let lexemes = match lex(text, FileId(0)) {
        Ok(lexemes) => lexemes,
        Err(err) => {
            let cut = err.span().start.min(text.len());
            lex(&text[..cut], FileId(0)).unwrap_or_default()
        }
    };
    let mut tokens: Vec<(Token, crate::dsl::span::Span)> = significant(&lexemes)
        .into_iter()
        .filter(|(_, span)| span.end <= offset)
        .collect();

    // A word the cursor touches is the one being typed; context comes from
    // what precedes it. (A fully-typed keyword lexes as that keyword, so
    // this drops keywords and idents alike.)
    if let Some((token, span)) = tokens.last()
        && span.end == offset
        && is_wordlike(token)
    {
        tokens.pop();
    }

    let mut cursor = Cursor {
        blocks: Vec::new(),
        in_list: false,
        in_at_parens: false,
        tokens: Vec::new(),
    };
    let mut list_depth = 0usize;
    let mut paren_depth = 0usize;

    let tokens: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LBrace => cursor.blocks.push(classify(&tokens[..i])),
            Token::RBrace => {
                cursor.blocks.pop();
            }
            Token::LBracket => list_depth += 1,
            Token::RBracket => list_depth = list_depth.saturating_sub(1),
            Token::LParen => {
                if paren_depth == 0 {
                    cursor.in_at_parens = matches!(tokens.get(i.wrapping_sub(1)), Some(Token::At));
                }
                paren_depth += 1;
            }
            Token::RParen => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    cursor.in_at_parens = false;
                }
            }
            _ => {}
        }
    }
    cursor.in_list = list_depth > 0;
    cursor.in_at_parens = cursor.in_at_parens && paren_depth > 0;
    cursor.tokens = tokens;
    cursor
}

/// Identify the block a `{` opens from the tokens before it: skip back
/// over the header (names, labels, coordinates) to the governing keyword.
fn classify(before: &[Token]) -> Block {
    for (i, token) in before.iter().enumerate().rev() {
        match token {
            Token::Component | Token::Extend => {
                let name = match before.get(i + 1) {
                    Some(Token::Ident(name)) => name.clone(),
                    _ => String::new(),
                };
                return Block::Component { name };
            }
            Token::View => {
                let kind = match before.get(i + 1) {
                    Some(Token::Ident(kind)) => Some(kind.clone()),
                    _ => None,
                };
                return Block::View { kind };
            }
            Token::Ports => {
                return Block::Ports {
                    instance: include_instance(&before[..i]),
                };
            }
            Token::Enclosure => return Block::Enclosure,
            Token::Cable => return Block::Cable,
            Token::Twisted => return Block::Twisted,
            Token::Connector => return Block::Connector,
            Token::ConnectorType => return Block::ConnectorType,
            // `grid`/`face` open a layout body only right after `layout`;
            // a view's `grid: 20;` never directly precedes a `{`.
            Token::Grid if matches!(before.get(i.wrapping_sub(1)), Some(Token::Layout)) => {
                return Block::LayoutGrid;
            }
            Token::Face if matches!(before.get(i.wrapping_sub(1)), Some(Token::Layout)) => {
                return Block::LayoutFace;
            }
            // Header tokens between the keyword and its `{`.
            Token::Ident(_)
            | Token::Str(_)
            | Token::Number(_)
            | Token::Colon
            | Token::Comma
            | Token::At
            | Token::LParen
            | Token::RParen => {}
            _ => return Block::Other,
        }
    }
    Block::Other
}

/// The instance a `ports { }` block belongs to: the ident right after the
/// nearest `include` in the same statement.
fn include_instance(before: &[Token]) -> Option<String> {
    for (i, token) in before.iter().enumerate().rev() {
        match token {
            Token::Include => {
                return match before.get(i + 1) {
                    Some(Token::Ident(name)) => Some(name.clone()),
                    _ => None,
                };
            }
            Token::Semicolon | Token::LBrace | Token::RBrace => return None,
            _ => {}
        }
    }
    None
}

fn is_wordlike(token: &Token) -> bool {
    !matches!(
        token,
        Token::Str(_)
            | Token::Number(_)
            | Token::LBrace
            | Token::RBrace
            | Token::LBracket
            | Token::RBracket
            | Token::LParen
            | Token::RParen
            | Token::Comma
            | Token::Semicolon
            | Token::Dot
            | Token::Colon
            | Token::Slash
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{project, resolve};

    /// Build an index from `files` (written to a temp project) and run
    /// completion on `live`, a version of `main.wb` with the cursor marked
    /// by `‸`. Returns the full items.
    fn complete_items_in(files: &[(&str, &str)], live: &str) -> Vec<CompletionItem> {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
        )
        .expect("write manifest");
        for (name, body) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create dirs");
            }
            std::fs::write(path, body).expect("write file");
        }
        let entry = dir.path().join("main.wb");
        let (project, problems) = project::load(&entry);
        let project = project.unwrap_or_else(|| panic!("project loads: {problems:?}"));
        let resolved = resolve::resolve(&project);
        let index = build_index(&project, &resolved);

        let offset = live.find('‸').expect("cursor marker");
        let text = live.replace('‸', "");
        let path = entry.canonicalize().expect("canonical entry");
        index.complete(&path, &text, offset)
    }

    /// [`complete_items_in`], labels only.
    fn complete_in(files: &[(&str, &str)], live: &str) -> Vec<String> {
        complete_items_in(files, live)
            .into_iter()
            .map(|i| i.label)
            .collect()
    }

    const LAMP: (&str, &str) = (
        "lamp.wb",
        "component Lamp {\n    pub port anode \"A+\";\n    pub port cathode \"A-\";\n    port heater \"H\";\n    connector plug \"Plug 2p\" {\n        pub port live \"L\" pin 1;\n        pub port neutral \"N\" pin 2;\n    }\n}\n",
    );

    const MAIN: (&str, &str) = (
        "main.wb",
        "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    pub port feed \"F\";\n    port internal \"I\";\n    wire red 1 [feed, l.anode];\n}\nview schematic \"S\" {\n    grid: 20;\n    include l at (4, 4) ports {\n        west: anode;\n    }\n}\n",
    );

    #[test]
    fn use_offers_auto_imports_with_relative_paths() {
        let battery = (
            "components/battery.wb",
            "component Battery { pub port hv_pos \"HV+\"; }\n",
        );
        let types = ("types.wb", "connector_type Can4p \"CAN 4p\" {\n}\n");
        let main = (
            "main.wb",
            "use Battery from \"components/battery.wb\";\nuse Can4p from \"types.wb\";\ncomponent Root { b: Battery; connector c: Can4p { } }\n",
        );
        let items = complete_items_in(&[battery, types, main], "use ‸\n");

        let battery = items
            .iter()
            .find(|i| i.label == "Battery")
            .expect("Battery offered");
        assert_eq!(
            battery.insert_text.as_deref(),
            Some("Battery from \"components/battery.wb\";")
        );
        let can = items
            .iter()
            .find(|i| i.label == "Can4p")
            .expect("connector type offered");
        assert_eq!(can.insert_text.as_deref(), Some("Can4p from \"types.wb\";"));
        // The doc's own definitions are not import candidates.
        assert!(!items.iter().any(|i| i.label == "Root"), "{items:?}");
    }

    #[test]
    fn extend_offers_top_level_components() {
        let labels = complete_in(&[LAMP, MAIN], "extend ‸\n");
        assert!(labels.contains(&"Root".to_string()), "{labels:?}");
        assert!(labels.contains(&"Lamp".to_string()), "{labels:?}");
    }

    #[test]
    fn connector_type_body_offers_layout_and_part() {
        let labels = complete_in(&[MAIN], "connector_type X \"X\" {\n    ‸\n}\n");
        assert!(labels.contains(&"layout".to_string()), "{labels:?}");
        assert!(labels.contains(&"part".to_string()), "{labels:?}");

        let labels = complete_in(&[MAIN], "connector_type X \"X\" {\n    layout ‸\n}\n");
        assert_eq!(labels, ["grid", "face"]);
    }

    #[test]
    fn layout_grid_body_offers_its_properties() {
        let labels = complete_in(
            &[MAIN],
            "connector_type X \"X\" {\n    layout grid {\n        ‸\n    }\n}\n",
        );
        assert_eq!(labels, ["rows", "cols", "numbering"]);
    }

    #[test]
    fn layout_face_body_offers_cavity_size_and_large() {
        let src = "connector_type X \"X\" {\n    layout face {\n        ‸\n    }\n}\n";
        assert_eq!(complete_in(&[MAIN], src), ["cavity"]);

        let src =
            "connector_type X \"X\" {\n    layout face {\n        cavity 1 at (0, 0) ‸\n    }\n}\n";
        assert_eq!(complete_in(&[MAIN], src), ["size"]);

        let src = "connector_type X \"X\" {\n    layout face {\n        cavity 1 at (0, 0) size ‸\n    }\n}\n";
        assert_eq!(complete_in(&[MAIN], src), ["large"]);
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        use std::path::Path;
        assert_eq!(
            relative_path(
                Path::new("/p/systems"),
                Path::new("/p/components/hv/inv.wb")
            ),
            "../components/hv/inv.wb"
        );
        assert_eq!(
            relative_path(Path::new("/p"), Path::new("/p/hv.wb")),
            "hv.wb"
        );
    }

    #[test]
    fn component_statement_start_offers_member_keywords() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    ‸\n}\n",
        );
        for expected in ["pub", "port", "wire", "cable", "connector", "component"] {
            assert!(
                labels.contains(&expected.to_string()),
                "{expected}: {labels:?}"
            );
        }
    }

    #[test]
    fn instantiation_colon_offers_types_in_scope() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: ‸\n}\n",
        );
        assert!(labels.contains(&"Lamp".to_string()), "{labels:?}");
        assert!(labels.contains(&"Root".to_string()), "{labels:?}");
    }

    #[test]
    fn endpoint_list_offers_instances_and_own_ports() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    pub port feed \"F\";\n    port internal \"I\";\n    wire red 1 [‸];\n}\n",
        );
        assert!(labels.contains(&"l".to_string()), "instance: {labels:?}");
        assert!(labels.contains(&"feed".to_string()), "own pub: {labels:?}");
        assert!(
            labels.contains(&"internal".to_string()),
            "own private ports are wirable bare: {labels:?}"
        );
    }

    #[test]
    fn endpoint_dot_offers_only_pub_ports_of_the_instance() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    wire red 1 [l.‸];\n}\n",
        );
        assert!(labels.contains(&"anode".to_string()), "{labels:?}");
        assert!(
            labels.contains(&"live".to_string()),
            "connector port: {labels:?}"
        );
        assert!(
            !labels.contains(&"heater".to_string()),
            "private port must not leak: {labels:?}"
        );
    }

    #[test]
    fn endpoint_dot_works_on_a_partially_typed_port() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    wire red 1 [l.an‸];\n}\n",
        );
        assert!(labels.contains(&"anode".to_string()), "{labels:?}");
    }

    #[test]
    fn connector_colon_offers_connector_types() {
        let files = [
            ("types.wb", "connector_type Can4p \"CAN 4p\" {\n}\n"),
            (
                "main.wb",
                "use Can4p from \"types.wb\";\ncomponent Root {\n    connector can: Can4p {\n        pub port a \"A\" pin 1;\n    }\n}\n",
            ),
        ];
        let labels = complete_in(
            &files,
            "use Can4p from \"types.wb\";\ncomponent Root {\n    connector can: ‸\n}\n",
        );
        assert!(labels.contains(&"Can4p".to_string()), "{labels:?}");
        assert!(!labels.contains(&"Root".to_string()), "{labels:?}");
    }

    #[test]
    fn typed_connector_body_offers_port_declarations() {
        let files = [
            ("types.wb", "connector_type Can4p \"CAN 4p\" {\n}\n"),
            (
                "main.wb",
                "use Can4p from \"types.wb\";\ncomponent Root {\n    connector can: Can4p {\n        pub port a \"A\" pin 1;\n    }\n}\n",
            ),
        ];
        let labels = complete_in(
            &files,
            "use Can4p from \"types.wb\";\ncomponent Root {\n    connector can: Can4p {\n        ‸\n    }\n}\n",
        );
        assert!(labels.contains(&"pub".to_string()), "{labels:?}");
        assert!(labels.contains(&"port".to_string()), "{labels:?}");
        assert!(
            !labels.contains(&"pin".to_string()),
            "the binding form is gone: {labels:?}"
        );
    }

    #[test]
    fn view_keyword_offers_kinds() {
        let labels = complete_in(&[LAMP, MAIN], "view ‸");
        assert_eq!(labels, ["schematic", "harness", "pinout"]);
    }

    #[test]
    fn schematic_include_offers_subject_instances() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n}\nview schematic \"S\" {\n    include ‸\n}\n",
        );
        assert_eq!(labels, ["l"]);
    }

    #[test]
    fn harness_include_dot_offers_instance_connectors() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n}\nview harness \"H\" {\n    include l.‸\n}\n",
        );
        assert_eq!(labels, ["plug"]);
    }

    #[test]
    fn ports_block_offers_sides_then_pub_ports() {
        let at_side = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n}\nview schematic \"S\" {\n    include l at (4, 4) ports {\n        ‸\n    }\n}\n",
        );
        assert!(at_side.contains(&"west".to_string()), "{at_side:?}");

        let at_port = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n}\nview schematic \"S\" {\n    include l at (4, 4) ports {\n        west: ‸\n    }\n}\n",
        );
        assert!(at_port.contains(&"anode".to_string()), "{at_port:?}");
        assert!(!at_port.contains(&"heater".to_string()), "{at_port:?}");
    }

    #[test]
    fn enclosure_offers_subject_pub_ports_and_sides_in_anchors() {
        let at_port = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    pub port feed \"F\";\n    port internal \"I\";\n}\nview schematic \"S\" {\n    enclosure {\n        ‸\n    }\n}\n",
        );
        assert!(at_port.contains(&"feed".to_string()), "{at_port:?}");
        assert!(!at_port.contains(&"internal".to_string()), "{at_port:?}");

        let at_anchor = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    pub port feed \"F\";\n}\nview schematic \"S\" {\n    enclosure {\n        feed at (‸\n    }\n}\n",
        );
        assert!(at_anchor.contains(&"west".to_string()), "{at_anchor:?}");
    }

    #[test]
    fn numbering_colon_offers_modes() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "connector_type C \"c\" {\n    layout: grid {\n        numbering: ‸\n    }\n}\n",
        );
        assert_eq!(labels, NUMBERING_MODES);
    }

    #[test]
    fn wire_keyword_offers_iec_colors() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    wire ‸\n}\n",
        );
        assert!(labels.contains(&"red".to_string()), "{labels:?}");
        assert!(labels.contains(&"turquoise".to_string()), "{labels:?}");
        assert!(
            !labels.contains(&"purple".to_string()),
            "synonyms are not canonical: {labels:?}"
        );
    }

    #[test]
    fn wire_color_completes_while_partially_typed() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    wire gr‸\n}\n",
        );
        assert!(labels.contains(&"green".to_string()), "{labels:?}");
    }

    #[test]
    fn tracer_slash_offers_iec_colors() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    wire green/‸\n}\n",
        );
        assert!(labels.contains(&"yellow".to_string()), "{labels:?}");
    }

    #[test]
    fn cable_statement_start_offers_twisted() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    cable c \"C\" {\n        ‸\n    }\n}\n",
        );
        for expected in ["wire", "twisted", "type", "length"] {
            assert!(
                labels.contains(&expected.to_string()),
                "{expected}: {labels:?}"
            );
        }
    }

    #[test]
    fn twisted_block_offers_wire() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    cable c \"C\" {\n        twisted {\n            ‸\n        }\n    }\n}\n",
        );
        assert_eq!(labels, ["wire"]);
    }

    #[test]
    fn endpoint_list_completes_inside_a_twisted_group() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    cable c \"C\" {\n        twisted {\n            wire white 0.5 [l.‸]\n        }\n    }\n}\n",
        );
        assert!(labels.contains(&"anode".to_string()), "{labels:?}");
    }

    #[test]
    fn merged_component_completes_across_fragments() {
        let files = [
            (
                "hv.wb",
                "extend Root {\n    h: Heater;\n    component Heater {\n        pub port coil \"C\";\n    }\n}\n",
            ),
            (
                "main.wb",
                "use Root from \"hv.wb\";\ncomponent Root {\n    pub port feed \"F\";\n    wire red 1 [feed, h.coil];\n}\n",
            ),
        ];
        // The instance `h` lives in hv.wb; completing in main.wb must see it.
        let labels = complete_in(
            &files,
            "use Root from \"hv.wb\";\ncomponent Root {\n    pub port feed \"F\";\n    wire red 1 [‸];\n}\n",
        );
        assert!(labels.contains(&"h".to_string()), "{labels:?}");
    }

    // ── the live-edit flow against an on-disk fixture project ──
    //
    // These mirror the server exactly: index from the pristine project,
    // then a rebuild with the mid-edit buffer shadowing the disk folded in
    // via `update_with` — the file being typed in usually fails to parse
    // and must keep its last-good scope. They run on the stable
    // `basic_project` fixture, never on `examples/` (the real vehicle
    // project, which changes freely).

    const FIXTURE_ENTRY: &str = "tests/fixtures/basic_project/main.wb";

    fn fixture_complete(doc: &str, live: &str) -> Vec<String> {
        let entry = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE_ENTRY)
            .canonicalize()
            .unwrap();
        let doc_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(doc)
            .canonicalize()
            .unwrap();

        let offset = live.find('‸').expect("cursor marker");
        let text = live.replace('‸', "");

        let (pristine, _) = project::load(&entry);
        let pristine = pristine.expect("fixture load");
        let mut index = build_index(&pristine, &resolve::resolve(&pristine));

        let mut overlay = crate::dsl::project::Overlay::default();
        overlay.insert(&doc_path, text.clone());
        let (live_project, _) = project::load_with(&entry, &overlay);
        let rebuilt = live_project
            .map(|p| build_index(&p, &resolve::resolve(&p)))
            .unwrap_or_default();
        index.update_with(rebuilt);

        index
            .complete(&doc_path, &text, offset)
            .into_iter()
            .map(|i| i.label)
            .collect()
    }

    #[test]
    fn mid_edit_entry_file_keeps_type_completion() {
        let disk =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ENTRY))
                .unwrap();
        let live = disk.replace(
            "    pack: Battery  \"Battery\";",
            "    x: ‸\n    pack: Battery  \"Battery\";",
        );
        assert_ne!(live, disk, "replacement target must exist in the fixture");
        let labels = fixture_complete(FIXTURE_ENTRY, &live);
        assert!(labels.iter().any(|l| l == "Battery"), "{labels:?}");
    }

    #[test]
    fn mid_edit_entry_file_keeps_instance_port_completion() {
        let disk =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ENTRY))
                .unwrap();
        let live = disk.replace(
            "    wire orange 50 \"HV+\" [pack.hv_pos, inv.dc_pos];",
            "    wire orange 50 \"HV+\" [pack.‸];",
        );
        assert_ne!(live, disk, "replacement target must exist in the fixture");
        let labels = fixture_complete(FIXTURE_ENTRY, &live);
        assert!(labels.iter().any(|l| l == "hv_pos"), "{labels:?}");
    }

    #[test]
    fn mid_edit_component_file_keeps_numbering_completion() {
        let doc = "tests/fixtures/basic_project/components/connectors.wb";
        let disk =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(doc)).unwrap();
        let live = disk.replace("        numbering: row_major;", "        numbering: ‸");
        assert_ne!(live, disk, "replacement target must exist in the fixture");
        let labels = fixture_complete(doc, &live);
        assert!(labels.iter().any(|l| l == "row_major"), "{labels:?}");
    }

    #[test]
    fn mid_edit_component_file_keeps_view_kind_completion() {
        let doc = "tests/fixtures/basic_project/components/inverter.wb";
        let disk =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(doc)).unwrap();
        let live = format!("{disk}\nview ‸\n");
        let labels = fixture_complete(doc, &live);
        assert!(labels.iter().any(|l| l == "schematic"), "{labels:?}");
    }

    #[test]
    fn lexically_broken_buffer_still_completes_before_the_break() {
        let labels = complete_in(
            &[LAMP, MAIN],
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    wire red 1 [l.‸]\n}\n@@@",
        );
        assert!(labels.contains(&"anode".to_string()), "{labels:?}");
    }
}
