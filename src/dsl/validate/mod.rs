//! Validation over the resolved registry.
//!
//! Most semantic checks already fire during resolution (undefined types,
//! duplicates, bad endpoints, private access, view includes) and
//! elaboration (containment cycles). This pass adds what's left:
//!
//! - wire arity (a wire needs at least two endpoints) — an error;
//! - pin number range (pins are positive integers) — an error;
//! - unused imports and pin assignments outside a connector — warnings,
//!   which only fail the run under `--strict`.
//!
//! (Unconnected-port detection is intentionally not done here: it needs
//! per-instance tree analysis and, on a full component library, floods
//! intentional unused-pin warnings — a separate, opt-in concern.)

use std::collections::HashMap;

use crate::dsl::ast::{CablePropertyValue, Member, Port, Wire};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::ColorName;
use crate::dsl::resolve::Resolved;
use crate::dsl::span::{FileId, Span};

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
                    for binding in &connector.pins {
                        if binding.pin.node == 0 {
                            problems.push(Problem::InvalidPin {
                                value: binding.pin.node,
                                src: src(),
                                at: binding.pin.span.into(),
                            });
                        }
                    }
                }
                Member::Cable(cable) => {
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
                            "length" if !matches!(p.value, CablePropertyValue::Number(_)) => {
                                problems.push(wrong("a number"));
                            }
                            "type" | "length" => {}
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
                "use ampseal from \"connectors.wb\";\ncomponent m { pub port a \"A\"; connector x1: ampseal { pin 1: a; } }\n",
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
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port a \"A\"; connector x1: ampseal { pin 0: a; } }\n",
        )]);
        assert!(
            codes.iter().any(|c| c == "wirebug::invalid_pin"),
            "{codes:?}"
        );
    }
}
