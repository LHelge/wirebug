//! Validation over the resolved registry.
//!
//! Most semantic checks already fire during resolution (undefined types,
//! duplicates, bad endpoints, private access, view includes) and
//! elaboration (containment cycles). This pass adds what's left:
//!
//! - wire arity (a wire needs at least two endpoints) — an error;
//! - pin number range (pins are positive integers) — an error;
//! - wire gauge and cable `length` (both must be positive) — errors;
//! - an endpoint repeated within one wire — an error;
//! - a cable with no conductors — a warning;
//! - unused imports and pin assignments outside a connector — warnings,
//!   which only fail the run under `--strict`.
//!
//! (Unconnected-port detection is intentionally not done here: it needs
//! per-instance tree analysis and, on a full component library, floods
//! intentional unused-pin warnings — a separate, opt-in concern.)

use std::collections::HashMap;

use crate::dsl::ast::{CablePropertyValue, Member, Port, Wire};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{ColorName, WireEnd};
use crate::dsl::resolve::Resolved;
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
                    validate_wire_colors(wire, def.file, resolved, &mut problems);
                    validate_gauge(wire, def.file, resolved, &mut problems);
                    validate_distinct_endpoints(wire, def.file, resolved, &mut problems);
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
                        validate_wire_colors(wire, def.file, resolved, &mut problems);
                        validate_gauge(wire, def.file, resolved, &mut problems);
                        validate_distinct_endpoints(wire, def.file, resolved, &mut problems);
                    }
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    for p in &cable.properties {
                        let key = p.key.node.as_str();
                        if let Some(&first) = seen.get(key) {
                            problems.push(Problem::DuplicateCableProperty {
                                key: key.to_string(),
                                src: src(),
                                at: p.key.span.into(),
                                first: first.into(),
                            });
                        } else {
                            seen.insert(key, p.key.span);
                        }
                        let wrong = |expected: &str| Problem::CablePropertyType {
                            key: key.to_string(),
                            expected: expected.to_string(),
                            src: src(),
                            at: p.value.span().into(),
                        };
                        match key {
                            "type" if !matches!(p.value, CablePropertyValue::Str(_)) => {
                                problems.push(wrong("a string"));
                            }
                            "length" => match &p.value {
                                CablePropertyValue::Number(length) => {
                                    validate_length(length, def.file, resolved, &mut problems);
                                }
                                _ => problems.push(wrong("a number")),
                            },
                            "type" => {}
                            _ => problems.push(Problem::UnknownCableProperty {
                                key: key.to_string(),
                                src: src(),
                                at: p.key.span.into(),
                            }),
                        }
                    }
                }
                _ => {}
            }
        }
    }

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
            if !instantiated && !merged {
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

/// A wire that lists one endpoint twice connects a port to itself. The
/// comparison is on the resolved [`WireEnd`], so an own port `gnd` and a
/// child's `ecu.gnd` stay distinct. Different wires sharing a port are not
/// compared: that is a deliberate double crimp.
fn validate_distinct_endpoints(
    wire: &Wire,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    let mut seen: HashMap<WireEnd, Span> = HashMap::new();
    for ep in &wire.endpoints {
        let end = WireEnd::from(ep);
        if let Some(&first) = seen.get(&end) {
            problems.push(Problem::DuplicateEndpoint {
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

/// A physical length in metres must be positive. Takes the spanned number
/// itself, not its owner, so any `length` value can be checked the same way.
fn validate_length(
    length: &Spanned<f64>,
    file: FileId,
    resolved: &Resolved,
    problems: &mut Vec<Problem>,
) {
    if length.node <= 0.0 {
        problems.push(Problem::InvalidLength {
            value: length.node,
            src: resolved.project.source(file),
            at: length.span.into(),
        });
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
