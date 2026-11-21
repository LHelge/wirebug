//! Validation over the resolved registry.
//!
//! Most semantic checks already fire during resolution (undefined types,
//! duplicates, bad endpoints, private access, view includes) and
//! elaboration (containment cycles). This pass adds what's left:
//!
//! - wire arity (a wire needs at least two endpoints) — an error;
//! - pin number range (pins are positive integers) — an error;
//! - the property bodies of cables and wires, and a component's part data
//!   (known keys, each once — across a component's `extend` fragments too —
//!   with the right kind of value) — errors;
//! - wire gauge and any `length` (both must be positive) — errors;
//! - an endpoint repeated within one wire or one shield — an error;
//! - a second shield in one cable body or one twisted pair, and a shield
//!   terminated on more than two ends — errors;
//! - a cable with no conductors — a warning;
//! - unused imports and pin assignments outside a connector — warnings,
//!   which only fail the run under `--strict`.
//!
//! (Unconnected-port detection is intentionally not done here: it needs
//! per-instance tree analysis and, on a full component library, floods
//! intentional unused-pin warnings — a separate, opt-in concern.)

use std::collections::HashMap;

use crate::dsl::ast::{
    Cable, Endpoint, Item, Member, Port, Property, PropertyValue, Quantity, Shield, Wire,
};
use crate::dsl::diagnostics::{
    EndpointOwner, FirstProperty, Problem, PropertyKind, PropertyOwner, ShieldOwner,
};
use crate::dsl::ir::{ColorName, WireEnd};
use crate::dsl::resolve::{DefId, Resolved};
use crate::dsl::span::{FileId, Span, Spanned};

/// Validate the resolved registry, returning any problems.
pub fn validate(resolved: &Resolved) -> Vec<Problem> {
    let mut problems = Vec::new();

    for def in &resolved.defs {
        let src = || resolved.project.source(def.file);
        for member in &def.ast.members {
            match member {
                Member::Wire(wire) => {
                    if wire.endpoints.len() < 2 {
                        problems.push(Problem::WireArity {
                            count: wire.endpoints.len(),
                            src: src(),
                            at: wire.span.into(),
                        });
                    }
                    validate_wire(wire, def.file, resolved, &mut problems);
                }
                Member::Port(port) if !port.pins.is_empty() => {
                    validate_pin_numbers(port, def.file, resolved, &mut problems);
                    problems.push(Problem::BarePortPin {
                        port: port.name.node.as_str().to_string(),
                        src: src(),
                        at: port.name.span.into(),
                    });
                }
                Member::Connector(connector) => {
                    for port in &connector.ports {
                        validate_pin_numbers(port, def.file, resolved, &mut problems);
                    }
                }
                Member::ConnectorInstance(connector) => {
                    for port in &connector.ports {
                        validate_pin_numbers(port, def.file, resolved, &mut problems);
                    }
                }
                Member::Cable(cable) => {
                    if cable.wires().next().is_none() {
                        problems.push(Problem::EmptyCable {
                            name: cable.name.node.as_str().to_string(),
                            src: src(),
                            at: cable.name.span.into(),
                        });
                    }
                    // A conductor is point-to-point: shared rails stay loose.
                    for wire in cable.wires() {
                        if wire.endpoints.len() != 2 {
                            problems.push(Problem::CableWireArity {
                                count: wire.endpoints.len(),
                                src: src(),
                                at: wire.span.into(),
                            });
                        }
                        validate_wire(wire, def.file, resolved, &mut problems);
                    }
                    validate_shields(
                        cable,
                        ShieldOwner::Cable,
                        &cable.shields,
                        def.file,
                        resolved,
                        &mut problems,
                    );
                    for pair in cable.twisted_groups() {
                        validate_shields(
                            cable,
                            ShieldOwner::Pair,
                            &pair.shields,
                            def.file,
                            resolved,
                            &mut problems,
                        );
                    }
                    validate_properties(
                        PropertyOwner::Cable,
                        &cable.properties,
                        def.file,
                        resolved,
                        &mut problems,
                    );
                }
                _ => {}
            }
        }
        validate_properties(
            PropertyOwner::Component,
            def.ast.properties(),
            def.file,
            resolved,
            &mut problems,
        );
    }
    validate_fragment_properties(resolved, &mut problems);

    for (fi, file) in resolved.project.files.iter().enumerate() {
        let fid = FileId(fi);
        for use_decl in &file.ast.uses {
            let name = use_decl.name.node.as_str();
            let instantiated = resolved.defs.iter().filter(|d| d.file == fid).any(|d| {
                d.instances
                    .values()
                    .any(|i| i.ast.type_name.node.as_str() == name)
                    || d.connectors
                        .values()
                        .any(|c| c.ast.type_name.node.as_str() == name)
                    || d.inlines.values().any(|i| {
                        i.ast
                            .halves
                            .iter()
                            .any(|h| h.type_name.node.as_str() == name)
                    })
            });
            // A fragment pull (`use Vehicle from "traction.wb";`) isn't
            // instantiated — it merges. It counts as used when this file owns a
            // same-named top-level definition that joined a merge group.
            let merged = resolved.defs.iter().enumerate().any(|(id, d)| {
                d.file == fid
                    && d.parent.is_none()
                    && d.name == name
                    && resolved.fragments(id).len() > 1
            });
            // A view's `for` documents the import, which is a use of it.
            let documented = file.ast.items.iter().any(|item| {
                matches!(item, Item::View(view)
                    if view.subject.as_ref().is_some_and(|s| s.node.as_str() == name))
            });
            if !instantiated && !merged && !documented {
                problems.push(Problem::UnusedImport {
                    name: name.to_string(),
                    src: resolved.project.source(fid),
                    at: use_decl.name.span.into(),
                });
            }
        }
    }

    problems
}

/// The checks every wire takes, loose or a cable's conductor: its colors,
/// gauge, endpoints and body.
fn validate_wire(wire: &Wire, file: FileId, resolved: &Resolved, problems: &mut Vec<Problem>) {
    validate_wire_colors(wire, file, resolved, problems);
    validate_gauge(wire, file, resolved, problems);
    validate_distinct_endpoints(
        EndpointOwner::Wire,
        &wire.endpoints,
        file,
        resolved,
        problems,
    );
    validate_properties(
        PropertyOwner::Wire,
        &wire.properties,
        file,
        resolved,
        problems,
    );
}

/// The shields of one cable body or one twisted pair (`owner`): at most
/// one, each terminated on at most two ends, none of them listed twice. A
/// shield end sharing a pin with a wire is a drain crimped alongside it,
/// and is not compared.
fn validate_shields(
    cable: &Cable,
    owner: ShieldOwner,
    shields: &[Shield],
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    if let [first, rest @ ..] = shields {
        for again in rest {
            problems.push(Problem::DuplicateShield {
                owner,
                cable: cable.name.node.as_str().to_string(),
                src: resolved.project.source(file),
                at: again.span.into(),
                first: first.span.into(),
            });
        }
    }
    for shield in shields {
        if shield.ends.len() > 2 {
            problems.push(Problem::ShieldArity {
                count: shield.ends.len(),
                src: resolved.project.source(file),
                at: shield.span.into(),
            });
        }
        validate_distinct_endpoints(
            EndpointOwner::Shield,
            &shield.ends,
            file,
            resolved,
            problems,
        );
    }
}

/// Check a property body against the keys its `owner` takes: an unknown
/// key, a key set twice, a value of the wrong kind, and a length that is
/// not positive are each reported, on the key or the value.
fn validate_properties<'a>(
    owner: PropertyOwner,
    properties: impl IntoIterator<Item = &'a Property>,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for p in properties {
        let key = p.key.node.as_str();
        if let Some(&first) = seen.get(key) {
            problems.push(Problem::DuplicateProperty {
                owner,
                key: key.to_string(),
                src: resolved.project.source(file),
                at: p.key.span.into(),
                first: first.into(),
            });
        } else {
            seen.insert(key, p.key.span);
        }
        let Some(expected) = owner.expects(key) else {
            problems.push(Problem::UnknownProperty {
                owner,
                key: key.to_string(),
                src: resolved.project.source(file),
                at: p.key.span.into(),
            });
            continue;
        };
        match (expected, &p.value) {
            (PropertyKind::Str, PropertyValue::Str(_)) => {}
            (PropertyKind::Length, PropertyValue::Number(length)) => {
                validate_length(length, file, resolved, problems);
            }
            _ => problems.push(Problem::PropertyType {
                owner,
                key: key.to_string(),
                expected,
                src: resolved.project.source(file),
                at: p.value.span().into(),
            }),
        }
    }
}

/// A component split with `extend` sets each property once across its
/// fragments: a key one fragment sets again after another set it is
/// reported against the first setting (the root `component`'s, when it has
/// one, since the root fragment comes first). A key set twice within one
/// fragment is [`validate_properties`]' to report.
fn validate_fragment_properties(resolved: &Resolved, problems: &mut Vec<Problem>) {
    for d in 0..resolved.defs.len() {
        let fragments = resolved.fragments(d);
        if fragments.len() < 2 || fragments[0] != d {
            continue;
        }
        let mut first: HashMap<&str, (DefId, Span)> = HashMap::new();
        for &frag in &fragments {
            let file = resolved.defs[frag].file;
            for p in resolved.defs[frag].ast.properties() {
                let key = p.key.node.as_str();
                let Some(&(set_in, span)) = first.get(key) else {
                    first.insert(key, (frag, p.key.span));
                    continue;
                };
                if set_in == frag {
                    continue;
                }
                problems.push(if span.file == file {
                    Problem::DuplicateProperty {
                        owner: PropertyOwner::Component,
                        key: key.to_string(),
                        src: resolved.project.source(file),
                        at: p.key.span.into(),
                        first: span.into(),
                    }
                } else {
                    Problem::DuplicateFragmentProperty {
                        key: key.to_string(),
                        src: resolved.project.source(file),
                        at: p.key.span.into(),
                        first: [FirstProperty {
                            key: key.to_string(),
                            src: resolved.project.source(span.file),
                            at: span.into(),
                        }],
                    }
                });
            }
        }
    }
}

/// Warn on a base or tracer color outside the IEC 60757 set. The wire
/// still renders (the name passes through verbatim), so this is a
/// warning — fatal only under `--strict`.
fn validate_wire_colors(
    wire: &Wire,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    let colors = std::iter::once(&wire.color).chain(&wire.tracer);
    for color in colors {
        if !ColorName::from(color.node.as_str()).is_standard() {
            problems.push(Problem::UnknownWireColor {
                name: color.node.as_str().to_string(),
                src: resolved.project.source(file),
                at: color.span.into(),
            });
        }
    }
}

/// A gauge is a conductor cross-section: zero or negative is nonsense.
fn validate_gauge(wire: &Wire, file: FileId, resolved: &Resolved, problems: &mut Vec<Problem>) {
    if wire.gauge.node <= 0.0 {
        problems.push(Problem::InvalidGauge {
            value: wire.gauge.node,
            src: resolved.project.source(file),
            at: wire.gauge.span.into(),
        });
    }
}

/// A wire (or a shield) that lists one endpoint twice connects a port to
/// itself. The comparison is on the resolved [`WireEnd`], so an own port
/// `gnd` and a child's `ecu.gnd` stay distinct. Different wires sharing a
/// port are not compared: that is a deliberate double crimp.
fn validate_distinct_endpoints(
    owner: EndpointOwner,
    endpoints: &[Endpoint],
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    let mut seen: HashMap<WireEnd, Span> = HashMap::new();
    for ep in endpoints {
        let end = WireEnd::from(ep);
        if let Some(&first) = seen.get(&end) {
            problems.push(Problem::DuplicateEndpoint {
                owner,
                endpoint: end.to_string(),
                src: resolved.project.source(file),
                at: ep.span.into(),
                first: first.into(),
            });
        } else {
            seen.insert(end, ep.span);
        }
    }
}

/// A physical length is in a length unit (metres when it has none) and
/// positive. Takes the spanned number itself, not its owner, so any
/// `length` value can be checked the same way.
fn validate_length(
    length: &Spanned<Quantity>,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    match (length.node.length_unit(), &length.node.unit) {
        (None, Some(unit)) => problems.push(Problem::UnknownLengthUnit {
            unit: unit.node.clone(),
            src: resolved.project.source(file),
            at: unit.span.into(),
        }),
        _ if length.node.length().is_none() => problems.push(Problem::InvalidLength {
            value: length.node.to_string(),
            src: resolved.project.source(file),
            at: length.span.into(),
        }),
        _ => {}
    }
}

fn validate_pin_numbers(
    port: &Port,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    for pin in &port.pins {
        if pin.node == 0 {
            problems.push(Problem::InvalidPin {
                value: pin.node,
                src: resolved.project.source(file),
                at: pin.span.into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::project::load;
    use crate::dsl::resolve::resolve;
    use miette::Diagnostic;

    fn validate_files(files: &[(&str, &str)]) -> Vec<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let (project, _) = load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = resolve(&project);
        validate(&resolved)
            .iter()
            .filter_map(|p| p.code().map(|c| c.to_string()))
            .collect()
    }

    #[test]
    fn fixture_project_has_no_validation_problems() {
        let main = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/main.wb"
        ));
        let (project, _) = load(&main);
        let project = project.expect("loads");
        let resolved = resolve(&project);
        let problems = validate(&resolved);
        assert!(
            problems.is_empty(),
            "unexpected validation problems: {problems:?}"
        );
    }

    #[test]
    fn wire_with_one_endpoint_errors() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { pub port a \"A\"; wire red 1 [a]; }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::wire_arity"),
            "{codes:?}"
        );
    }

    /// A single component with the given cable body, plus three own ports the
    /// cable wires can land on.
    fn cable(body: &str) -> Vec<String> {
        validate_files(&[(
            "main.wb",
            &format!(
                "component m {{ pub port a \"A\"; pub port b \"B\"; pub port c \"C\"; cable cab {{ {body} }} }}\n"
            ),
        )])
    }

    #[test]
    fn cable_wire_with_three_endpoints_errors() {
        let codes = cable("wire red 1 [a, b, c];");
        assert!(
            codes.iter().any(|c| c == "wirebug::cable_wire_arity"),
            "{codes:?}"
        );
    }

    #[test]
    fn two_endpoint_cable_wire_is_clean() {
        let codes = cable("type: \"Twisted pair\"; length: 2.5; wire red 1 [a, b];");
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn unknown_cable_property_errors() {
        let codes = cable("color: \"red\"; wire red 1 [a, b];");
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_cable_property"),
            "{codes:?}"
        );
    }

    #[test]
    fn cable_length_must_be_a_number_errors() {
        let codes = cable("length: \"long\"; wire red 1 [a, b];");
        assert!(
            codes.iter().any(|c| c == "wirebug::cable_property_type"),
            "{codes:?}"
        );
    }

    #[test]
    fn zero_and_negative_cable_length_error() {
        for length in ["0", "-2", "-0.5"] {
            let codes = cable(&format!("length: {length}; wire red 1 [a, b];"));
            assert_eq!(codes, ["wirebug::invalid_length"], "length {length}");
        }
    }

    #[test]
    fn fractional_cable_length_is_clean() {
        let codes = cable("length: 0.1; wire red 1 [a, b];");
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn string_cable_length_is_a_type_error_not_a_length_error() {
        let codes = cable("length: \"-2\"; wire red 1 [a, b];");
        assert_eq!(codes, ["wirebug::cable_property_type"]);
    }

    #[test]
    fn zero_and_negative_loose_wire_gauge_error() {
        for gauge in ["0", "-1", "-0.25"] {
            let codes = validate_files(&[(
                "main.wb",
                &format!(
                    "component m {{ pub port a \"A\"; pub port b \"B\"; wire black {gauge} [a, b]; }}\n"
                ),
            )]);
            assert_eq!(codes, ["wirebug::invalid_gauge"], "gauge {gauge}");
        }
    }

    #[test]
    fn non_positive_cable_conductor_gauge_errors() {
        let codes = cable("wire red 0 [a, b]; twisted { wire red -1 [a, b]; wire blue 1 [a, c]; }");
        assert_eq!(codes, ["wirebug::invalid_gauge", "wirebug::invalid_gauge"]);
    }

    #[test]
    fn fractional_gauge_is_clean() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; wire black 0.25 [a, b]; }\n",
        )]);
        assert!(codes.is_empty(), "{codes:?}");
    }

    /// The source text that each label of each problem in a single-file
    /// project points at.
    fn labelled_text(src: &str) -> Vec<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = resolve(&project);
        validate(&resolved)
            .iter()
            .flat_map(|p| p.labels().into_iter().flatten())
            .map(|l| src[l.offset()..l.offset() + l.len()].to_string())
            .collect()
    }

    #[test]
    fn invalid_gauge_points_at_the_number() {
        let labels = labelled_text(
            "component m { pub port a \"A\"; pub port b \"B\"; wire black -1 [a, b]; }\n",
        );
        assert_eq!(labels, ["-1"]);
    }

    #[test]
    fn invalid_length_points_at_the_number() {
        let labels = labelled_text(
            "component m { pub port a \"A\"; pub port b \"B\"; cable w { length: -2; wire red 1 [a, b]; } }\n",
        );
        assert_eq!(labels, ["-2"]);
    }

    #[test]
    fn lengths_in_each_unit_are_clean() {
        let codes = cable(
            "length: 2m; wire red 1 [a, b] { length: 150mm; } wire red 1 [a, c] { length: 15cm; }",
        );
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn an_unknown_length_unit_errors_on_the_unit() {
        let src = "component m { pub port a \"A\"; pub port b \"B\"; cable w { length: 2in; wire red 1 [a, b] { length: 3km; } } }\n";
        assert_eq!(labelled_text(src), ["km", "in"]);
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let problems = validate(&resolve(&project.expect("loads")));
        let report: Vec<String> = problems
            .iter()
            .map(|p| {
                let code = p.code().map(|c| c.to_string()).unwrap_or_default();
                let help = p.help().map(|h| h.to_string()).unwrap_or_default();
                format!("{code}: {p} | {help}")
            })
            .collect();
        insta::assert_snapshot!(report.join("\n"), @r"
        wirebug::unknown_length_unit: unknown length unit `km` | write a length as a positive number of metres (`length: 2.5;`), or with one of the units `m`, `cm` or `mm` directly after the number (`length: 150mm;`)
        wirebug::unknown_length_unit: unknown length unit `in` | write a length as a positive number of metres (`length: 2.5;`), or with one of the units `m`, `cm` or `mm` directly after the number (`length: 150mm;`)
        ");
    }

    #[test]
    fn a_non_positive_length_with_a_unit_errors_on_the_number_and_its_unit() {
        let labels = labelled_text(
            "component m { pub port a \"A\"; pub port b \"B\"; cable w { length: -150mm; wire red 1 [a, b] { length: 0cm; } } }\n",
        );
        assert_eq!(labels, ["0cm", "-150mm"]);
        let codes = cable("length: -150mm; wire red 1 [a, b];");
        assert_eq!(codes, ["wirebug::invalid_length"]);
    }

    #[test]
    fn a_length_under_a_micrometre_is_not_positive() {
        let codes = cable("length: 0.0001mm; wire red 1 [a, b];");
        assert_eq!(codes, ["wirebug::invalid_length"]);
    }

    #[test]
    fn a_unit_on_a_string_key_is_a_type_error() {
        let codes = cable("type: 2m; wire red 1 [a, b] { part: 5mm; }");
        assert_eq!(
            codes,
            [
                "wirebug::cable_property_type",
                "wirebug::cable_property_type"
            ]
        );
    }

    #[test]
    fn invalid_length_in_a_type_instantiated_twice_reports_once() {
        let codes = validate_files(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; pub port b \"B\"; cable w { length: 0; wire red 1 [a, b]; } }\n\
             component m { l1: leaf; l2: leaf; }\n",
        )]);
        assert_eq!(codes, ["wirebug::invalid_length"]);
    }

    /// A leaf type with two `pub` ports and a parent `m` around one instance
    /// `l` of it, with own ports `a` and `gnd`; `body` goes inside `m`.
    fn with_leaf(body: &str) -> String {
        format!(
            "component leaf {{ pub port a \"A\"; pub port gnd \"GND\"; }}\n\
             component m {{ pub port a \"A\"; pub port gnd \"GND\"; l: leaf; {body} }}\n"
        )
    }

    #[test]
    fn repeated_endpoint_in_one_wire_errors() {
        let codes = validate_files(&[("main.wb", &with_leaf("wire black 1 [l.gnd, l.gnd];"))]);
        assert_eq!(codes, ["wirebug::duplicate_endpoint"]);
    }

    #[test]
    fn repeated_own_port_in_one_wire_errors() {
        let codes = validate_files(&[("main.wb", &with_leaf("wire black 1 [gnd, l.a, gnd];"))]);
        assert_eq!(codes, ["wirebug::duplicate_endpoint"]);
    }

    #[test]
    fn duplicate_endpoint_labels_both_occurrences() {
        let src = with_leaf("wire black 1 [l.gnd, a, l.gnd];");
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), &src).expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let problems = validate(&resolved);
        let [problem] = problems.as_slice() else {
            panic!("expected one problem, got {problems:?}");
        };
        assert!(problem.is_error());
        assert_eq!(problem.to_string(), "wire lists endpoint `l.gnd` twice");
        let first = src.find("l.gnd").expect("first occurrence");
        let again = src.rfind("l.gnd").expect("second occurrence");
        let mut offsets: Vec<(usize, usize)> = problem
            .labels()
            .into_iter()
            .flatten()
            .map(|l| (l.offset(), l.len()))
            .collect();
        offsets.sort_unstable();
        assert_eq!(offsets, [(first, 5), (again, 5)]);
    }

    #[test]
    fn endpoint_listed_three_times_reports_each_repeat() {
        let codes = validate_files(&[("main.wb", &with_leaf("wire black 1 [a, l.a, a, a];"))]);
        assert_eq!(
            codes,
            ["wirebug::duplicate_endpoint", "wirebug::duplicate_endpoint"]
        );
    }

    #[test]
    fn own_port_and_child_port_of_the_same_name_are_distinct_endpoints() {
        let codes = validate_files(&[("main.wb", &with_leaf("wire black 1 [gnd, l.gnd];"))]);
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn two_wires_on_one_port_are_a_double_crimp_not_a_problem() {
        let codes = validate_files(&[(
            "main.wb",
            &with_leaf("wire black 1 [l.gnd, gnd]; wire black 1 [l.gnd, a];"),
        )]);
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn repeated_endpoint_in_a_cable_conductor_errors() {
        let codes = cable("twisted { wire red 1 [a, b]; wire blue 1 [c, c]; }");
        assert_eq!(codes, ["wirebug::duplicate_endpoint"]);
    }

    #[test]
    fn repeated_endpoint_in_a_type_instantiated_twice_reports_once() {
        let codes = validate_files(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; wire red 1 [a, a]; }\n\
             component m { l1: leaf; l2: leaf; }\n",
        )]);
        assert_eq!(codes, ["wirebug::duplicate_endpoint"]);
    }

    #[test]
    fn cable_without_conductors_warns() {
        for body in ["", "type: \"Twisted pair\"; length: 1.5;"] {
            let codes = cable(body);
            assert_eq!(codes, ["wirebug::empty_cable"], "body {body:?}");
        }
    }

    #[test]
    fn empty_cable_is_a_warning_labelled_on_its_name() {
        let src = "component m { cable spare \"Spare\" { } }\n";
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let problems = validate(&resolved);
        let [problem] = problems.as_slice() else {
            panic!("expected one problem, got {problems:?}");
        };
        assert!(!problem.is_error());
        assert_eq!(labelled_text(src), ["spare"]);
    }

    #[test]
    fn shields_terminated_on_none_one_or_two_ends_are_clean() {
        for shield in ["shield;", "shield [c];", "shield \"SHLD\" [a, c];"] {
            let codes = cable(&format!("wire red 1 [a, b]; {shield}"));
            assert!(codes.is_empty(), "{shield}: {codes:?}");
        }
    }

    #[test]
    fn an_overall_shield_and_a_pair_shield_are_two_screens_not_a_repeat() {
        let codes = cable(
            "shield [c]; twisted { wire red 1 [a, b]; wire blue 1 [a, c]; shield [c]; } \
             twisted { wire red 1 [b, a]; wire blue 1 [c, a]; shield; }",
        );
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn a_drain_sharing_a_pin_with_a_wire_is_not_a_problem() {
        let codes = cable("wire black 1 [a, b]; shield [a];");
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn a_second_shield_in_a_cable_or_a_pair_errors_naming_both() {
        let src = "component m { pub port a \"A\"; pub port b \"B\"; \
                   cable cab { wire red 1 [a, b]; shield [a]; shield \"S2\"; \
                   twisted { wire red 1 [b, a]; wire blue 1 [a, b]; shield; shield [b]; } } }\n";
        assert_eq!(
            validate_files(&[("main.wb", src)]),
            ["wirebug::duplicate_shield", "wirebug::duplicate_shield"]
        );
        assert_eq!(
            labelled_text(src),
            [r#"shield "S2";"#, "shield [a];", "shield [b];", "shield;"]
        );
    }

    #[test]
    fn duplicate_shield_messages_name_what_the_shield_screens() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("main.wb"),
            "component m { pub port a \"A\"; pub port b \"B\"; cable can { \
             wire red 1 [a, b]; shield; shield; \
             twisted { wire red 1 [b, a]; wire blue 1 [a, b]; shield; shield; } } }\n",
        )
        .expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let messages: Vec<String> = validate(&resolved).iter().map(|p| p.to_string()).collect();
        assert_eq!(
            messages,
            [
                "cable `can` has more than one shield",
                "a twisted pair of cable `can` has more than one shield",
            ]
        );
    }

    #[test]
    fn a_shield_with_three_ends_errors_on_the_shield() {
        let src = "component m { pub port a \"A\"; pub port b \"B\"; pub port c \"C\"; \
                   cable cab { wire red 1 [a, b]; shield [a, b, c]; } }\n";
        assert_eq!(
            validate_files(&[("main.wb", src)]),
            ["wirebug::shield_arity"]
        );
        assert_eq!(labelled_text(src), ["shield [a, b, c];"]);
    }

    #[test]
    fn a_shield_listing_an_end_twice_is_a_duplicate_endpoint() {
        let src = "component m { pub port a \"A\"; pub port b \"B\"; \
                   cable cab { twisted { wire red 1 [a, b]; wire blue 1 [b, a]; shield [b, b]; } } }\n";
        assert_eq!(
            validate_files(&[("main.wb", src)]),
            ["wirebug::duplicate_endpoint"]
        );
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let problems = validate(&resolved);
        assert_eq!(problems[0].to_string(), "shield lists endpoint `b` twice");
    }

    #[test]
    fn a_cable_of_nothing_but_a_shield_is_still_empty() {
        assert_eq!(cable("shield [a];"), ["wirebug::empty_cable"]);
    }

    #[test]
    fn twisted_group_of_two_is_clean() {
        let codes = cable("twisted { wire red 1 [a, b]; wire blue 1 [a, c]; }");
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn twisted_group_wires_keep_cable_checks() {
        // Arity and color checks reach conductors inside a group too.
        let codes = cable("twisted { wire red 1 [a, b, c]; wire greeen 1 [a, b]; }");
        assert!(
            codes.iter().any(|c| c == "wirebug::cable_wire_arity"),
            "{codes:?}"
        );
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_wire_color"),
            "{codes:?}"
        );
    }

    #[test]
    fn twisted_as_property_key_is_unknown() {
        // The old `twisted: true;` property form no longer exists; the
        // block form replaced it.
        let codes = cable("type: \"x\"; wire red 1 [a, b];");
        assert!(codes.is_empty(), "{codes:?}");
        let codes = cable("lay: 5; wire red 1 [a, b];");
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_cable_property"),
            "{codes:?}"
        );
    }

    #[test]
    fn duplicate_cable_property_errors() {
        let codes = cable("length: 1; length: 2; wire red 1 [a, b];");
        assert!(
            codes
                .iter()
                .any(|c| c == "wirebug::duplicate_cable_property"),
            "{codes:?}"
        );
    }

    /// A component with own ports `a` and `b` and one loose wire between
    /// them, with `body` as the wire's body.
    fn wire_body(body: &str) -> Vec<String> {
        validate_files(&[(
            "main.wb",
            &format!(
                "component m {{ pub port a \"A\"; pub port b \"B\"; wire red 1 [a, b] {{ {body} }} }}\n"
            ),
        )])
    }

    #[test]
    fn wire_body_with_length_type_and_part_is_clean() {
        let codes = wire_body("length: 1.2; type: \"FLRY-B\"; part: \"W-1\";");
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn unknown_wire_property_reuses_the_cable_property_code() {
        let codes = wire_body("colour: \"red\";");
        assert_eq!(codes, ["wirebug::unknown_cable_property"]);
    }

    #[test]
    fn duplicate_wire_property_reuses_the_cable_property_code() {
        let codes = wire_body("part: \"A\"; part: \"B\";");
        assert_eq!(codes, ["wirebug::duplicate_cable_property"]);
    }

    #[test]
    fn wrongly_typed_wire_properties_reuse_the_cable_property_code() {
        for body in ["length: \"long\";", "type: 5;", "part: abc;"] {
            assert_eq!(
                wire_body(body),
                ["wirebug::cable_property_type"],
                "body {body:?}"
            );
        }
    }

    #[test]
    fn non_positive_wire_length_errors() {
        for length in ["0", "-1.5"] {
            let codes = wire_body(&format!("length: {length};"));
            assert_eq!(codes, ["wirebug::invalid_length"], "length {length}");
        }
    }

    #[test]
    fn part_is_a_wire_property_but_not_a_cable_property() {
        let codes = cable("part: \"C-1\"; wire red 1 [a, b] { part: \"W-1\"; }");
        assert_eq!(codes, ["wirebug::unknown_cable_property"]);
    }

    #[test]
    fn cable_conductor_bodies_are_checked_inside_a_twisted_pair_too() {
        let codes = cable(
            "length: 2; wire red 1 [a, b] { length: 3; } \
             twisted { wire red 1 [a, b] { length: 0; } wire blue 1 [a, c] { lenght: 1; } }",
        );
        assert_eq!(
            codes,
            ["wirebug::invalid_length", "wirebug::unknown_cable_property"]
        );
    }

    #[test]
    fn wire_property_problems_name_the_wire() {
        let messages = |src: &str| -> Vec<String> {
            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join("main.wb"), src).expect("write");
            let (project, _) = load(&dir.path().join("main.wb"));
            let resolved = resolve(project.as_ref().expect("loads"));
            validate(&resolved)
                .iter()
                .map(|p| match p.help() {
                    Some(help) => format!("{p} | {help}"),
                    None => p.to_string(),
                })
                .collect()
        };
        let wire = messages(
            "component m { pub port a \"A\"; pub port b \"B\"; \
             wire red 1 [a, b] { colour: 1; part: 2; part: \"x\"; } }\n",
        );
        insta::assert_snapshot!(wire.join("\n"), @r#"
        unknown wire property `colour` | a wire supports `length: <number>;`, `type: "...";` and `part: "...";`
        wire property `part` expects a string
        duplicate wire property `part`
        "#);
        // A cable's wording is unchanged.
        let cable = messages(
            "component m { pub port a \"A\"; pub port b \"B\"; \
             cable k { colour: 1; length: \"x\"; type: \"t\"; type: \"u\"; wire red 1 [a, b]; } }\n",
        );
        insta::assert_snapshot!(cable.join("\n"), @r#"
        unknown cable property `colour` | a cable supports `type: "...";` and `length: <number>;`
        cable property `length` expects a number
        duplicate cable property `type`
        "#);
    }

    /// A component `m` with `body` among its members.
    fn component_body(body: &str) -> Vec<String> {
        validate_files(&[(
            "main.wb",
            &format!("component m {{ pub port a \"A\"; {body} }}\n"),
        )])
    }

    #[test]
    fn component_part_data_is_clean() {
        let codes = component_body(
            "part: \"EV200AAANA\"; manufacturer: \"TE\"; description: \"Main contactor\"; rating: \"500 A\";",
        );
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn component_property_problems_reuse_the_cable_property_codes() {
        assert_eq!(
            component_body("colour: \"red\";"),
            ["wirebug::unknown_cable_property"]
        );
        assert_eq!(
            component_body("part: \"A\"; pub port b \"B\"; part: \"B\";"),
            ["wirebug::duplicate_cable_property"]
        );
        assert_eq!(
            component_body("rating: 500;"),
            ["wirebug::cable_property_type"]
        );
    }

    #[test]
    fn component_property_problems_name_the_component() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("main.wb"),
            "component m { length: 2; rating: 500; part: \"A\"; part: \"B\"; }\n",
        )
        .expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let messages: Vec<String> = validate(&resolved)
            .iter()
            .map(|p| match p.help() {
                Some(help) => format!("{p} | {help}"),
                None => p.to_string(),
            })
            .collect();
        insta::assert_snapshot!(messages.join("\n"), @r#"
        unknown component property `length` | a component supports `part: "...";`, `manufacturer: "...";`, `description: "...";` and `rating: "...";`; to place an instance, name its type after the colon: `length: <Type> "<label>";`
        component property `rating` expects a string
        duplicate component property `part`
        "#);
    }

    #[test]
    fn an_instance_without_its_type_is_told_to_name_the_type() {
        // `l: "Lamp";` parses as part data, a string after the colon; the
        // help covers the instance the author more likely meant.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), "component m { l: \"Lamp\"; }\n")
            .expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let problems = validate(&resolved);
        let [problem] = problems.as_slice() else {
            panic!("expected one problem, got {problems:?}");
        };
        assert_eq!(problem.to_string(), "unknown component property `l`");
        let help = problem.help().expect("a help").to_string();
        assert!(
            help.ends_with(
                "; to place an instance, name its type after the colon: `l: <Type> \"<label>\";`"
            ),
            "{help}"
        );
    }

    #[test]
    fn component_part_data_in_a_type_instantiated_twice_reports_once() {
        let codes = validate_files(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; colour: \"red\"; }\n\
             component m { l1: leaf; l2: leaf; }\n",
        )]);
        assert_eq!(codes, ["wirebug::unknown_cable_property"]);
    }

    /// A component `M` whose root sets `root` and whose two `extend`
    /// fragments, in their own files, set `one` and `two`.
    fn split_component(root: &str, one: &str, two: &str) -> Vec<Problem> {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = [
            (
                "main.wb",
                format!(
                    "use M from \"one.wb\";\nuse M from \"two.wb\";\ncomponent M {{ pub port a \"A\"; {root} }}\n"
                ),
            ),
            ("one.wb", format!("extend M {{ {one} }}\n")),
            ("two.wb", format!("extend M {{ {two} }}\n")),
        ];
        for (name, body) in &files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        assert!(
            resolved.problems.is_empty(),
            "resolve: {:?}",
            resolved.problems
        );
        validate(&resolved)
    }

    /// Each problem as `<code> @ <file>` plus a `first @ <file>` per related
    /// note: where the repeat and the first setting are reported.
    fn placed(problems: &[Problem]) -> Vec<String> {
        problems
            .iter()
            .map(|p| {
                let file = |d: &dyn Diagnostic| {
                    d.source_code()
                        .and_then(|src| {
                            let label = d.labels()?.next()?;
                            let contents = src.read_span(label.inner(), 0, 0).ok()?;
                            let name = std::path::Path::new(contents.name()?).file_name()?;
                            Some(name.to_string_lossy().into_owned())
                        })
                        .unwrap_or_default()
                };
                let mut line = format!("{} @ {}", p.code().expect("a code"), file(p));
                for related in p.related().into_iter().flatten() {
                    line.push_str(&format!(" | first @ {}", file(related)));
                }
                line
            })
            .collect()
    }

    #[test]
    fn an_extend_fragment_cannot_set_again_a_property_the_root_set() {
        let problems = split_component("part: \"P-1\";", "part: \"P-2\";", "");
        let [problem] = problems.as_slice() else {
            panic!("expected one problem, got {problems:?}");
        };
        assert_eq!(
            placed(&problems),
            ["wirebug::duplicate_cable_property @ one.wb | first @ main.wb"]
        );
        assert_eq!(problem.to_string(), "duplicate component property `part`");
    }

    #[test]
    fn two_extend_fragments_cannot_both_set_a_property() {
        let problems = split_component("", "rating: \"500 A\";", "rating: \"400 A\";");
        assert_eq!(
            placed(&problems),
            ["wirebug::duplicate_cable_property @ two.wb | first @ one.wb"]
        );
    }

    #[test]
    fn an_extend_fragment_may_set_a_property_the_root_left_unset() {
        let problems = split_component(
            "part: \"P-1\";",
            "manufacturer: \"TE\";",
            "rating: \"500 A\";",
        );
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn a_repeat_inside_one_fragment_is_reported_once() {
        // The fragment's own duplicate is the per-body check's; the
        // cross-fragment check does not report it again.
        let problems = split_component("", "part: \"A\"; part: \"B\";", "");
        assert_eq!(
            placed(&problems),
            ["wirebug::duplicate_cable_property @ one.wb"]
        );
    }

    #[test]
    fn fragments_in_one_file_report_a_repeat_with_two_labels() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("main.wb"),
            "component M { part: \"P-1\"; }\nextend M { part: \"P-2\"; }\n",
        )
        .expect("write");
        let (project, _) = load(&dir.path().join("main.wb"));
        let resolved = resolve(project.as_ref().expect("loads"));
        let problems = validate(&resolved);
        let [Problem::DuplicateProperty { owner, .. }] = problems.as_slice() else {
            panic!("expected one two-label duplicate, got {problems:?}");
        };
        assert_eq!(*owner, PropertyOwner::Component);
    }

    #[test]
    fn invalid_wire_length_in_a_type_instantiated_twice_reports_once() {
        let codes = validate_files(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; pub port b \"B\"; wire red 1 [a, b] { length: 0; } }\n\
             component m { l1: leaf; l2: leaf; }\n",
        )]);
        assert_eq!(codes, ["wirebug::invalid_length"]);
    }

    #[test]
    fn non_iec_wire_color_warns() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; wire chartreuse 1 [a, b]; }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_wire_color"),
            "{codes:?}"
        );
    }

    #[test]
    fn non_iec_tracer_color_warns() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; wire green/yellowish 1 [a, b]; }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_wire_color"),
            "{codes:?}"
        );
    }

    #[test]
    fn non_iec_cable_conductor_color_warns() {
        let codes = cable("wire beige 1 [a, b];");
        assert!(
            codes.iter().any(|c| c == "wirebug::unknown_wire_color"),
            "{codes:?}"
        );
    }

    #[test]
    fn iec_colors_and_synonyms_are_clean() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; wire purple 1 [a, b]; wire gray/gold 1 [a, b]; }\n",
        )]);
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn unused_import_warns() {
        let codes = validate_files(&[
            ("main.wb", "use leaf from \"leaf.wb\";\ncomponent m { }\n"),
            ("leaf.wb", "component leaf { pub port a \"A\"; }\n"),
        ]);
        assert!(
            codes.iter().any(|c| c == "wirebug::unused_import"),
            "{codes:?}"
        );
    }

    #[test]
    fn component_import_named_only_by_a_view_for_does_not_warn() {
        let codes = validate_files(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { }\nview pinout \"Leaf\" for leaf { include j at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector j \"J\" { pub port a \"A\" pin 1; } }\n",
            ),
        ]);
        assert!(
            !codes.iter().any(|c| c == "wirebug::unused_import"),
            "{codes:?}"
        );
    }

    #[test]
    fn connector_type_import_used_by_connector_instance_does_not_warn() {
        let codes = validate_files(&[
            (
                "main.wb",
                "use ampseal from \"connectors.wb\";\ncomponent m { connector x1: ampseal { pub port a \"A\" pin 1; } }\n",
            ),
            (
                "connectors.wb",
                "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\n",
            ),
        ]);
        assert!(
            !codes.iter().any(|c| c == "wirebug::unused_import"),
            "{codes:?}"
        );
    }

    #[test]
    fn connector_type_import_used_by_an_inline_half_does_not_warn() {
        let codes = validate_files(&[
            (
                "main.wb",
                "use ampseal from \"connectors.wb\";\ncomponent m { inline ic { male: ampseal; port a \"A\" pin 1; } }\n",
            ),
            (
                "connectors.wb",
                "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\n",
            ),
        ]);
        assert!(
            !codes.iter().any(|c| c == "wirebug::unused_import"),
            "{codes:?}"
        );
    }

    #[test]
    fn bare_port_pin_warns() {
        let codes = validate_files(&[("main.wb", "component m { pub port a \"A\" pin 1; }\n")]);
        assert!(
            codes.iter().any(|c| c == "wirebug::bare_port_pin"),
            "{codes:?}"
        );
    }

    #[test]
    fn connector_pin_zero_errors() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { connector j1 \"J1\" { pub port a \"A\" pin 0; } }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::invalid_pin"),
            "{codes:?}"
        );
    }

    #[test]
    fn ganged_pin_zero_errors() {
        let codes = validate_files(&[(
            "main.wb",
            "component m { connector j1 \"J1\" { pub port a \"A\" pins [1, 0, 2]; } }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::invalid_pin"),
            "{codes:?}"
        );
    }

    #[test]
    fn connector_instance_pin_zero_errors() {
        let codes = validate_files(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pub port a \"A\" pin 0; } }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::invalid_pin"),
            "{codes:?}"
        );
    }
}
