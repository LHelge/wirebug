//! Validation over the resolved registry.
//!
//! Most semantic checks already fire during resolution (undefined types,
//! duplicates, bad endpoints, private access, view includes) and
//! elaboration (containment cycles). This pass adds what's left:
//!
//! - wire arity (a wire needs at least two endpoints) — an error;
//! - unused imports and pin assignments outside a connector — warnings,
//!   which only fail the run under `--strict`.
//!
//! (Unconnected-port detection is intentionally not done here: it needs
//! per-instance tree analysis and, on a full component library, floods
//! intentional unused-pin warnings — a separate, opt-in concern.)

use std::collections::HashMap;

use crate::dsl::ast::{CablePropertyValue, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::resolve::Resolved;
use crate::dsl::span::{FileId, Span};

/// Validate the resolved registry, returning any problems.
pub fn validate(resolved: &Resolved) -> Vec<Problem> {
    let mut problems = Vec::new();

    for def in &resolved.defs {
        let src = || resolved.project.source(def.file);
        for member in &def.ast.members {
            match member {
                Member::Wire(wire) if wire.endpoints.len() < 2 => {
                    problems.push(Problem::WireArity {
                        count: wire.endpoints.len(),
                        src: src(),
                        at: wire.span.into(),
                    });
                }
                Member::Port(port) if !port.pins.is_empty() => {
                    problems.push(Problem::BarePortPin {
                        port: port.name.node.as_str().to_string(),
                        src: src(),
                        at: port.name.span.into(),
                    });
                }
                Member::Cable(cable) => {
                    // A conductor is point-to-point: shared rails stay loose.
                    for wire in &cable.wires {
                        if wire.endpoints.len() != 2 {
                            problems.push(Problem::CableWireArity {
                                count: wire.endpoints.len(),
                                src: src(),
                                at: wire.span.into(),
                            });
                        }
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
            let used = resolved.defs.iter().filter(|d| d.file == fid).any(|d| {
                d.instances
                    .values()
                    .any(|i| i.ast.type_name.node.as_str() == name)
            });
            if !used {
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
    fn seed_project_has_no_validation_problems() {
        let main =
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/main.wb"));
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
    fn unused_import_warns() {
        let codes = validate_files(&[
            ("main.wb", "use leaf from \"leaf.wb\"\ncomponent m { }\n"),
            ("leaf.wb", "component leaf { pub port a \"A\"; }\n"),
        ]);
        assert!(
            codes.iter().any(|c| c == "wirebug::unused_import"),
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
}
