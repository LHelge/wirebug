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

    /// `extend <name>` was written for a component that no reachable file
    /// introduces with `component <name>`.
    #[error("`extend {name}` has no `component {name}` to extend")]
    #[diagnostic(
        code(wirebug::orphan_fragment),
        help(
            "introduce the component once with `component {name} {{ … }}`; other files add to it with `extend {name}`"
        )
    )]
    OrphanFragment {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("nothing to extend")]
        at: SourceSpan,
    },

    /// `extend` was used on a definition nested inside another component.
    #[error("`extend` is only allowed at the top level, not nested")]
    #[diagnostic(
        code(wirebug::nested_extend),
        help(
            "nested definitions are private to their parent; only a top-level component can be split into `extend` fragments"
        )
    )]
    NestedExtend {
        #[source_code]
        src: NamedSource<String>,
        #[label("nested `extend`")]
        at: SourceSpan,
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

    /// Two includes of one view draw boxes that overlap. Box sizes follow
    /// from ports, labels and pin rows in the renderer's layout, so this is
    /// found by laying the view out (`render::lint`), not from the DSL.
    #[error("`{include}` overlaps `{other}` in view \"{view}\"")]
    #[diagnostic(
        code(wirebug::overlapping_includes),
        help(
            "boxes are sized from their ports and labels; move one include so the boxes no longer overlap (touching edges are fine)"
        )
    )]
    OverlappingIncludes {
        include: String,
        other: String,
        view: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("overlaps `{other}`")]
        at: SourceSpan,
        #[label("`{other}` is placed here")]
        first: SourceSpan,
    },

    /// A schematic include sits so close to another that a wired port has
    /// no room for its wire: the router leaves each port by one grid step
    /// and keeps one step clear of every box, so that stub lands inside the
    /// other box's clearance and the wire cannot be routed.
    #[error("no room to route {ports}: `{include}` is too close to `{other}` in view \"{view}\"")]
    #[diagnostic(
        code(wirebug::include_too_close),
        help(
            "a wire leaves its port by one grid step and keeps one grid step clear of every other box, so leave at least two grid steps between a wired port and the box it faces"
        )
    )]
    IncludeTooClose {
        include: String,
        other: String,
        view: String,
        /// The blocked ports, as a list of backticked `inst.port`s.
        ports: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("too close to `{other}`")]
        at: SourceSpan,
        #[label("`{other}` is placed here")]
        first: SourceSpan,
    },

    /// A schematic wire the router finds no path for, though both its
    /// ports have room to leave: boxes less than two grid steps apart close
    /// the gaps between them, walling one end in. Labelled on the includes
    /// the wire ends on (an enclosure end has none).
    #[error("no route for the wire from `{from}` to `{to}` in view \"{view}\"")]
    #[diagnostic(
        code(wirebug::unroutable_wire),
        help(
            "wires keep one grid step clear of every box, so boxes less than two grid steps apart leave no gap between them; open a gap of at least two grid steps in the boxes around the walled-in end"
        )
    )]
    UnroutableWire {
        from: String,
        to: String,
        view: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("the wire ends here")]
        at: SourceSpan,
        #[label("and here")]
        other: Option<SourceSpan>,
    },

    /// A view's `grid:` step is zero or negative, so no renderer can draw
    /// it. Found by asking the view's renderer (`render::lint`), like the
    /// other layout problems.
    #[error("grid step must be positive, got {value}")]
    #[diagnostic(
        code(wirebug::invalid_grid),
        help(
            "the grid step is drawing units per grid cell, e.g. `grid: 20;`; omit `grid:` for the default"
        )
    )]
    InvalidGrid {
        value: f64,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a positive step")]
        at: SourceSpan,
    },

    /// A view's `grid:` step is finer than its renderer can draw: ports sit
    /// two steps apart, and that pitch must clear a port label. The
    /// `minimum` is the renderer's own (`render::lint` asks it).
    #[error(
        "grid step {value} is too fine for a {kind} view; ports need at least {minimum} per step"
    )]
    #[diagnostic(
        code(wirebug::grid_too_fine),
        help(
            "ports sit two grid steps apart, so a finer grid overlaps their labels; use `grid: {minimum};` or coarser, or omit `grid:` for the default"
        )
    )]
    GridTooFine {
        value: f64,
        minimum: f64,
        kind: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("finer than {minimum}")]
        at: SourceSpan,
    },

    /// A conductor between connectors of two of a component's children that
    /// none of the component's harness views draws: no view includes both
    /// ends' connectors, so the build drawings miss it. A warning: the
    /// design is sound, its drawings incomplete.
    #[error("{conductor} from `{from}` to `{to}` is in no harness view of `{component}`")]
    #[diagnostic(severity(Warning), code(wirebug::undrawn_conductor))]
    UndrawnConductor {
        /// `wire` or ``cable `<name>` conductor``, with its quoted label
        /// when it has one.
        conductor: String,
        from: String,
        to: String,
        component: String,
        /// The includes that would draw it, or, for an inline connector
        /// that declares no half, that it must declare one first.
        #[help]
        advice: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("drawn in no harness view")]
        at: SourceSpan,
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

    /// A connector (inline or typed) claims the same physical pin twice.
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

    /// An `inline` body property line uses a key other than `male`/`female`.
    #[error("unknown inline connector property `{key}`")]
    #[diagnostic(
        code(wirebug::unknown_inline_property),
        help(
            "an inline body takes `male: <ConnectorType>;` and `female: <ConnectorType>;` before its ports"
        )
    )]
    UnknownInlineProperty {
        key: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such property")]
        at: SourceSpan,
    },

    /// An `inline` declares the same housing half twice.
    #[error("duplicate inline connector half `{half}`")]
    #[diagnostic(code(wirebug::duplicate_inline_half))]
    DuplicateInlineHalf {
        half: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("declared again here")]
        at: SourceSpan,
        #[label("first declared here")]
        first: SourceSpan,
    },

    /// A harness include selects a non-half of an inline connector.
    #[error("`{found}` is not a half of inline connector `{inline}`")]
    #[diagnostic(
        code(wirebug::unknown_inline_half),
        help("write `include {inline}.male` or `include {inline}.female`")
    )]
    UnknownInlineHalf {
        found: String,
        inline: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such half")]
        at: SourceSpan,
    },

    /// A harness include selects a half the inline never declared.
    #[error("inline connector `{inline}` does not declare a `{half}` half")]
    #[diagnostic(
        code(wirebug::undeclared_inline_half),
        help("declare `{half}: <ConnectorType>;` in `inline {inline}`")
    )]
    UndeclaredInlineHalf {
        half: String,
        inline: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("half not declared")]
        at: SourceSpan,
    },

    /// `pub` on an inline connector port has no effect: inline ports are
    /// only addressable within the defining component.
    #[error("`pub` has no effect on an inline connector port")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::inline_pub_port),
        help("inline connector ports are always addressable within the defining component")
    )]
    InlinePubPort {
        #[source_code]
        src: NamedSource<String>,
        #[label("this `pub`")]
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

    /// One wire lists the same endpoint twice: it connects a port to
    /// itself. (Two *different* wires on one port are a deliberate double
    /// crimp and are fine.)
    #[error("wire lists endpoint `{endpoint}` twice")]
    #[diagnostic(
        code(wirebug::duplicate_endpoint),
        help("remove the repeat, or name the endpoint this wire was meant to reach")
    )]
    DuplicateEndpoint {
        endpoint: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("listed again here")]
        at: SourceSpan,
        #[label("first listed here")]
        first: SourceSpan,
    },

    /// A cable carries no conductors. Nothing breaks (a harness view just
    /// draws no cable box for it), so this is a warning.
    #[error("cable `{name}` has no conductors")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::empty_cable),
        help("add its conductors as `wire <color> <gauge> [a, b];` lines, or remove the cable")
    )]
    EmptyCable {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("this cable")]
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

    /// A wire's gauge (its cross-section in mm²) is zero or negative.
    #[error("wire gauge must be positive, got {value}")]
    #[diagnostic(
        code(wirebug::invalid_gauge),
        help("the gauge is the conductor cross-section in mm², e.g. `0.5` or `4`")
    )]
    InvalidGauge {
        value: f64,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a positive cross-section")]
        at: SourceSpan,
    },

    /// A `length` (in metres) is zero or negative.
    #[error("length must be positive, got {value}")]
    #[diagnostic(
        code(wirebug::invalid_length),
        help("a length is in metres, e.g. `length: 2.5;`")
    )]
    InvalidLength {
        value: f64,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a positive length")]
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

    /// A view names a kind no renderer implements.
    #[error("unknown view kind `{kind}`")]
    #[diagnostic(
        code(wirebug::unknown_view_kind),
        help("supported view kinds are `schematic`, `harness`, and `pinout`")
    )]
    UnknownViewKind {
        kind: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a supported view kind")]
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

    /// Two views in the project share a title. Both still render (the
    /// repeat to a suffixed file), but the index shows two identically named
    /// sections. The first view is a related note, since it may sit in
    /// another file than the repeat.
    #[error("duplicate view title \"{title}\"")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::duplicate_view_title),
        help("give each view its own title; the repeat otherwise renders to a `_2`-suffixed file")
    )]
    DuplicateViewTitle {
        title: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("titled again here")]
        at: SourceSpan,
        #[related]
        first: [FirstViewTitle; 1],
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

    /// A view documents a component the design never instantiates, so
    /// there is no instance to draw it against.
    #[error("view \"{view}\" documents `{subject}`, which the design never instantiates")]
    #[diagnostic(
        code(wirebug::uninstantiated_view_subject),
        help(
            "a view is drawn against an instance of its file's top-level component; instantiate `{subject}` under the root component, or remove the view"
        )
    )]
    UninstantiatedViewSubject {
        view: String,
        subject: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("`{subject}` has no instance")]
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

    /// A wire color outside the IEC 60757 vocabulary. It still renders
    /// (stroke and code pass through verbatim), but it has no standard
    /// code and won't match real wire stock.
    #[error("unknown wire color `{name}`")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::unknown_wire_color),
        help(
            "use an IEC 60757 color: black, brown, red, orange, yellow, green, \
             blue, violet, grey, white, pink, turquoise, gold, silver"
        )
    )]
    UnknownWireColor {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not an IEC 60757 color name")]
        at: SourceSpan,
    },
}

/// The first view to use a title, attached to [`Problem::DuplicateViewTitle`]
/// as a related note: a miette diagnostic carries one source file, and the
/// two views may be in different files.
#[derive(Debug, Error, Diagnostic)]
#[error("first view titled \"{title}\"")]
#[diagnostic(severity(Advice))]
pub struct FirstViewTitle {
    pub title: String,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("first titled here")]
    pub at: SourceSpan,
}

impl Problem {
    /// True for problems that fail the run (severity error). Warnings
    /// return false unless the caller is running `--strict`.
    pub fn is_error(&self) -> bool {
        !matches!(self.severity(), Some(miette::Severity::Warning))
    }
}
