//! miette diagnostics for the check pipeline.
//!
//! One growing enum, [`Problem`], with a variant per failure class. Each
//! carries the offending file's source (so miette can render the snippet)
//! and a label span. Severity defaults to error; warnings set it
//! explicitly. The enum grows as later phases (resolve, elaborate,
//! validate) land.

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Problem {
    /// A lexical error (bad character, unterminated string, …).
    #[error("{message}")]
    #[diagnostic(code(wirebug::lex))]
    Lex {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    /// A syntax error from the parser.
    #[error("{message}")]
    #[diagnostic(code(wirebug::parse))]
    Parse {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    /// A `use` referenced a file that couldn't be found.
    #[error("cannot find imported file `{target}`")]
    #[diagnostic(
        code(wirebug::use_not_found),
        help("paths in `use` are relative to the importing file")
    )]
    UseNotFound {
        target: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("imported here")]
        at: SourceSpan,
    },

    /// No `wirebug.toml` was found while discovering the project.
    #[error("no wirebug project found: no `wirebug.toml` in `{start}` or any parent directory")]
    #[diagnostic(
        code(wirebug::no_project),
        help("a wirebug project is a directory containing a `wirebug.toml` manifest")
    )]
    NoProject { start: String },

    /// Failed to read a source file from disk.
    #[error("failed to read `{path}`: {source}")]
    #[diagnostic(code(wirebug::io))]
    Io {
        path: String,
        source: std::io::Error,
    },

    /// The project directory has no `wirebug.toml`.
    #[error("no project manifest: `wirebug.toml` is missing from `{dir}`")]
    #[diagnostic(
        code(wirebug::manifest_missing),
        help(
            "create `wirebug.toml` beside `main.wb` with a `[project]` table (`name` and `version` required)"
        )
    )]
    ManifestMissing { dir: String },

    /// `wirebug.toml` failed to parse, or didn't match the schema.
    #[error("{message}")]
    #[diagnostic(code(wirebug::manifest_parse))]
    ManifestParse {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    // --- Resolution ---
    /// An instance names a component type that isn't in scope.
    #[error("unknown component type `{name}`")]
    #[diagnostic(
        code(wirebug::undefined_type),
        help("define `{name}`, or `use` it from another file")
    )]
    UndefinedType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a component in scope")]
        at: SourceSpan,
    },

    /// A component connector instance names a connector type that isn't in
    /// scope.
    #[error("unknown connector type `{name}`")]
    #[diagnostic(
        code(wirebug::undefined_connector_type),
        help("define `{name}` as a top-level `connector_type`, or `use` it from another file")
    )]
    UndefinedConnectorType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a connector type in scope")]
        at: SourceSpan,
    },

    /// A `use` resolved to a file that has no matching top-level component
    /// or connector type.
    #[error("`{name}` is not a top-level component or connector type in `{file}`")]
    #[diagnostic(code(wirebug::unresolved_import))]
    UnresolvedImport {
        name: String,
        file: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such component to import")]
        at: SourceSpan,
    },

    /// Two connector types share a name in one file's connector-type scope.
    #[error("duplicate connector type `{name}`")]
    #[diagnostic(code(wirebug::duplicate_connector_type))]
    DuplicateConnectorType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("redefined here")]
        at: SourceSpan,
        #[label("first defined here")]
        first: SourceSpan,
    },

    /// Two component types share a name in one file's scope.
    #[error("duplicate component type `{name}`")]
    #[diagnostic(code(wirebug::duplicate_type))]
    DuplicateType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("redefined here")]
        at: SourceSpan,
        #[label("first defined here")]
        first: SourceSpan,
    },

    /// Two instances in one component share a name.
    #[error("duplicate instance name `{name}`")]
    #[diagnostic(code(wirebug::duplicate_instance))]
    DuplicateInstance {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("reused here")]
        at: SourceSpan,
        #[label("first used here")]
        first: SourceSpan,
    },

    /// Two ports in one component share a name (connectors are not
    /// namespaces — port names are unique across the whole component).
    #[error("duplicate port name `{name}`")]
    #[diagnostic(
        code(wirebug::duplicate_port),
        help("port names must be unique across a component, including across connectors")
    )]
    DuplicatePort {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("redeclared here")]
        at: SourceSpan,
        #[label("first declared here")]
        first: SourceSpan,
    },

    /// A wire endpoint names an instance that doesn't exist in the
    /// enclosing component.
    #[error("unknown instance `{name}`")]
    #[diagnostic(code(wirebug::unknown_instance))]
    UnknownInstance {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not an instance in this component")]
        at: SourceSpan,
    },

    /// A wire endpoint names a port that doesn't exist on its component.
    #[error("unknown port `{port}`{on}")]
    #[diagnostic(code(wirebug::unknown_port))]
    UnknownPort {
        port: String,
        /// e.g. " on `cell_module`", or empty for a self port.
        on: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such port")]
        at: SourceSpan,
    },

    /// A wire endpoint references a non-`pub` port from outside its owner.
    #[error("port `{port}` of `{ty}` is not `pub`")]
    #[diagnostic(
        code(wirebug::private_port),
        help("mark it `pub` in `{ty}`, or wire it through a pub port")
    )]
    PrivatePort {
        port: String,
        ty: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("referenced from outside `{ty}`")]
        at: SourceSpan,
    },

    /// A view `include` names something that isn't an instance of the
    /// component the view documents.
    #[error("unknown instance `{name}` in view")]
    #[diagnostic(code(wirebug::unknown_include))]
    UnknownInclude {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not an instance of the documented component")]
        at: SourceSpan,
    },

    /// A view can't be bound to a single documented component (the file
    /// has zero or several top-level components).
    #[error("cannot tell which component this view documents")]
    #[diagnostic(
        code(wirebug::view_subject),
        help("a view documents the file's single top-level component")
    )]
    ViewSubject {
        #[source_code]
        src: NamedSource<String>,
        #[label("this view")]
        at: SourceSpan,
    },

    /// A view port placement names a side that isn't a compass direction.
    #[error("unknown side `{found}`")]
    #[diagnostic(
        code(wirebug::unknown_port_side),
        help("a port side is one of `north`, `east`, `south`, `west`")
    )]
    UnknownPortSide {
        found: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a side")]
        at: SourceSpan,
    },

    /// A view places the same port twice in one include's `ports { }` block.
    #[error("duplicate port `{port}` in view")]
    #[diagnostic(code(wirebug::duplicate_view_port))]
    DuplicateViewPort {
        port: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("placed again here")]
        at: SourceSpan,
        #[label("first placed here")]
        first: SourceSpan,
    },

    /// A view includes the same render target twice. Schematic views key this
    /// by instance; harness views key it by `instance.connector`.
    #[error("duplicate include `{target}` in view")]
    #[diagnostic(
        code(wirebug::duplicate_view_include),
        help("place each view include target only once")
    )]
    DuplicateViewInclude {
        target: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("included again here")]
        at: SourceSpan,
        #[label("first included here")]
        first: SourceSpan,
    },

    /// Two connectors in one component share a designator.
    #[error("duplicate connector designator `{name}`")]
    #[diagnostic(
        code(wirebug::duplicate_connector_name),
        help("connector designators must be unique within a component")
    )]
    DuplicateConnectorName {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("reused here")]
        at: SourceSpan,
        #[label("first used here")]
        first: SourceSpan,
    },

    /// A component connector binds the same physical pin more than once.
    #[error("duplicate pin `{pin}` in connector `{connector}`")]
    #[diagnostic(code(wirebug::duplicate_connector_pin))]
    DuplicateConnectorPin {
        pin: u32,
        connector: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("bound again here")]
        at: SourceSpan,
        #[label("first bound here")]
        first: SourceSpan,
    },

    /// A port was assigned to two different component connectors.
    #[error("port `{port}` is already assigned to another connector")]
    #[diagnostic(
        code(wirebug::port_connector_conflict),
        help("a component port can belong to only one physical connector")
    )]
    PortConnectorConflict {
        port: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("assigned again here")]
        at: SourceSpan,
    },

    /// Two cables in one component share a designator.
    #[error("duplicate cable designator `{name}`")]
    #[diagnostic(
        code(wirebug::duplicate_cable_name),
        help("cable designators must be unique within a component")
    )]
    DuplicateCableName {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("reused here")]
        at: SourceSpan,
        #[label("first used here")]
        first: SourceSpan,
    },

    /// A wire inside a `cable` block does not have exactly two endpoints.
    /// A cable conductor is point-to-point; shared rails stay loose wires.
    #[error("a cable wire must connect exactly two endpoints, found {count}")]
    #[diagnostic(
        code(wirebug::cable_wire_arity),
        help("split a shared rail into one loose `wire` per net, or add a junction")
    )]
    CableWireArity {
        count: usize,
        #[source_code]
        src: NamedSource<String>,
        #[label("this cable wire")]
        at: SourceSpan,
    },

    /// A cable property uses a key other than `type` or `length`.
    #[error("unknown cable property `{key}`")]
    #[diagnostic(
        code(wirebug::unknown_cable_property),
        help("a cable supports `type: \"...\";` and `length: <number>;`")
    )]
    UnknownCableProperty {
        key: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such property")]
        at: SourceSpan,
    },

    /// A cable sets the same property key twice.
    #[error("duplicate cable property `{key}`")]
    #[diagnostic(code(wirebug::duplicate_cable_property))]
    DuplicateCableProperty {
        key: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("set again here")]
        at: SourceSpan,
        #[label("first set here")]
        first: SourceSpan,
    },

    /// A cable property's value is the wrong kind (e.g. `length: "x"`).
    #[error("cable property `{key}` expects {expected}")]
    #[diagnostic(code(wirebug::cable_property_type))]
    CablePropertyType {
        key: String,
        /// e.g. "a number" or "a string".
        expected: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("wrong value type")]
        at: SourceSpan,
    },

    /// A harness include names a connector designator that the included
    /// instance's type doesn't declare.
    #[error("unknown connector `{name}`{on}")]
    #[diagnostic(
        code(wirebug::unknown_connector),
        help("give the connector a designator: `connector {name} \"...\" {{ ... }}`")
    )]
    UnknownConnector {
        name: String,
        /// e.g. " on `obc_dcdc`".
        on: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such connector")]
        at: SourceSpan,
    },

    /// An include uses the wrong shape for its view kind: a harness include
    /// without a connector or with a `ports { }` block, or a schematic
    /// include carrying a connector segment.
    #[error("{message}")]
    #[diagnostic(code(wirebug::wrong_include_form), help("{help}"))]
    WrongIncludeForm {
        message: String,
        help: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    /// A view declares `grid` or `enclosure` more than once. The body takes
    /// items in any order, but at most one of each.
    #[error("view declares `{kind}` more than once")]
    #[diagnostic(
        code(wirebug::duplicate_view_item),
        help("a view takes at most one `grid` and one `enclosure` block")
    )]
    DuplicateViewItem {
        kind: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("declared again here")]
        at: SourceSpan,
    },

    /// A view places two text boxes with the same name.
    #[error("duplicate text box `{name}` in view")]
    #[diagnostic(code(wirebug::duplicate_view_text))]
    DuplicateViewText {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("reused here")]
        at: SourceSpan,
        #[label("first used here")]
        first: SourceSpan,
    },

    /// Text boxes are currently schematic-only.
    #[error("text boxes are not supported in `{kind}` views")]
    #[diagnostic(
        code(wirebug::unsupported_view_text),
        help("move this `text` item to a schematic view")
    )]
    UnsupportedViewText {
        kind: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("schematic views only")]
        at: SourceSpan,
    },

    /// An enclosure port's `at (x, y)` anchor isn't exactly one side keyword
    /// and one coordinate: two coordinates, two sides, or a side in the
    /// wrong slot (west/east must be the x slot, north/south the y slot).
    #[error("{message}")]
    #[diagnostic(
        code(wirebug::enclosure_anchor),
        help("place a port as `<port> at (west|east, <y>)` or `<port> at (<x>, north|south)`")
    )]
    EnclosureAnchor {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    // --- Elaboration ---
    /// `main.wb` doesn't define exactly one top-level component to elaborate.
    #[error("`main.wb` must define exactly one top-level component")]
    #[diagnostic(
        code(wirebug::no_root),
        help("the design root is the single top-level component in `main.wb`")
    )]
    NoRoot,

    /// A component instantiates itself, directly or transitively.
    #[error("component `{name}` contains itself: {cycle}")]
    #[diagnostic(
        code(wirebug::containment_cycle),
        help("a component cannot instantiate itself, directly or through its children")
    )]
    ContainmentCycle {
        name: String,
        cycle: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("this component")]
        at: SourceSpan,
    },

    // --- Validation ---
    /// A wire has fewer than two endpoints.
    #[error("a wire needs at least two endpoints, found {count}")]
    #[diagnostic(code(wirebug::wire_arity))]
    WireArity {
        count: usize,
        #[source_code]
        src: NamedSource<String>,
        #[label("this wire")]
        at: SourceSpan,
    },

    /// A pin number is not a positive integer. The parser accepts `u32` so
    /// validation can point at the exact authored pin token.
    #[error("pin numbers must be positive, got {value}")]
    #[diagnostic(code(wirebug::invalid_pin))]
    InvalidPin {
        value: u32,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a positive pin number")]
        at: SourceSpan,
    },

    /// An imported component is never instantiated in the importing file.
    #[error("unused import `{name}`")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::unused_import),
        help("remove the `use`, or instantiate `{name}`")
    )]
    UnusedImport {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("never instantiated")]
        at: SourceSpan,
    },

    /// A pin assignment on a port that isn't inside a `connector` block.
    #[error("pin assignment on `{port}`, which is not inside a connector")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::bare_port_pin),
        help("pins are connector metadata; put the port inside a `connector` block")
    )]
    BarePortPin {
        port: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("pin here has no connector")]
        at: SourceSpan,
    },
}

impl Problem {
    /// True for problems that fail the run (severity error). Warnings
    /// return false unless the caller is running `--strict`.
    pub fn is_error(&self) -> bool {
        !matches!(self.severity(), Some(miette::Severity::Warning))
    }
}
