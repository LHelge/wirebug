//! Reference binding: the flat-namespace lookups (instances, inlines,
//! ports — merge-group aware) and the passes that bind every reference to
//! its declaration: instance types, connector-instance and inline-half
//! connector types, and wire endpoints.

use std::collections::HashMap;

use crate::dsl::ast::{self, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{ConnectorName, InstanceName, PortName};
use crate::dsl::span::{FileId, Span};

use super::{ConnectorTypeId, DefId, InlineFacts, InstFacts, PortFacts, Resolver};

impl<'a> Resolver<'a> {
    /// Find an instance by name across every fragment of `d`'s merged
    /// component (the flat namespace).
    pub(super) fn lookup_instance(&self, d: DefId, name: &InstanceName) -> Option<&InstFacts<'a>> {
        self.groups
            .fragments(d)
            .into_iter()
            .find_map(|f| self.defs[f].instances.get(name))
    }

    /// Find an inline connector by name across every fragment of `d`'s
    /// merged component (inlines share the instance namespace).
    pub(super) fn lookup_inline(&self, d: DefId, name: &InstanceName) -> Option<&InlineFacts<'a>> {
        self.groups
            .fragments(d)
            .into_iter()
            .find_map(|f| self.defs[f].inlines.get(name))
    }

    /// Find a port by name across every fragment of `d`'s merged component.
    pub(super) fn lookup_port(&self, d: DefId, name: &PortName) -> Option<&PortFacts<'a>> {
        self.groups
            .fragments(d)
            .into_iter()
            .find_map(|f| self.defs[f].ports.get(name))
    }

    pub(super) fn resolve_instances(&mut self, envs: &[HashMap<String, DefId>]) {
        for (d, env) in envs.iter().enumerate() {
            let pending: Vec<(InstanceName, String, Span)> = self.defs[d]
                .instances
                .iter()
                .map(|(name, facts)| {
                    (
                        name.clone(),
                        facts.ast.type_name.node.as_str().to_string(),
                        facts.ast.type_name.span,
                    )
                })
                .collect();
            let file = self.defs[d].file;
            for (name, type_name, span) in pending {
                match env.get(&type_name) {
                    Some(&tid) => {
                        if let Some(facts) = self.defs[d].instances.get_mut(&name) {
                            facts.type_id = Some(tid);
                        }
                    }
                    None => self.problems.push(Problem::UndefinedType {
                        name: type_name,
                        src: self.project.source(file),
                        at: span.into(),
                    }),
                }
            }
        }
    }

    pub(super) fn resolve_connector_instances(
        &mut self,
        connector_type_scope: &HashMap<FileId, HashMap<String, ConnectorTypeId>>,
    ) {
        for d in 0..self.defs.len() {
            let pending: Vec<(ConnectorName, String, Span)> = self.defs[d]
                .connectors
                .iter()
                .map(|(name, facts)| {
                    (
                        name.clone(),
                        facts.ast.type_name.node.as_str().to_string(),
                        facts.ast.type_name.span,
                    )
                })
                .collect();
            let file = self.defs[d].file;
            let env = &connector_type_scope[&file];
            for (name, type_name, span) in pending {
                match env.get(&type_name) {
                    Some(&tid) => {
                        if let Some(facts) = self.defs[d].connectors.get_mut(&name) {
                            facts.type_id = Some(tid);
                        }
                    }
                    None => self.problems.push(Problem::UndefinedConnectorType {
                        name: type_name,
                        src: self.project.source(file),
                        at: span.into(),
                    }),
                }
            }
        }
    }

    /// Resolve each inline's `male:`/`female:` housing-half lines: validate
    /// the keys and bind each half's connector type through the defining
    /// file's connector-type scope.
    pub(super) fn resolve_inline_halves(
        &mut self,
        connector_type_scope: &HashMap<FileId, HashMap<String, ConnectorTypeId>>,
    ) {
        for d in 0..self.defs.len() {
            let file = self.defs[d].file;
            let env = &connector_type_scope[&file];
            let inline_names: Vec<InstanceName> = self.defs[d].inlines.keys().cloned().collect();
            for name in inline_names {
                let inline_ast = self.defs[d].inlines[&name].ast;
                let mut seen: HashMap<&str, Span> = HashMap::new();
                for half in &inline_ast.halves {
                    let key = half.key.node.as_str();
                    if key != "male" && key != "female" {
                        self.problems.push(Problem::UnknownInlineProperty {
                            key: key.to_string(),
                            src: self.project.source(file),
                            at: half.key.span.into(),
                        });
                        continue;
                    }
                    if let Some(&first) = seen.get(key) {
                        self.problems.push(Problem::DuplicateInlineHalf {
                            half: key.to_string(),
                            src: self.project.source(file),
                            at: half.key.span.into(),
                            first: first.into(),
                        });
                        continue;
                    }
                    seen.insert(key, half.key.span);
                    let type_id = env.get(half.type_name.node.as_str()).copied();
                    if type_id.is_none() {
                        self.problems.push(Problem::UndefinedConnectorType {
                            name: half.type_name.node.as_str().to_string(),
                            src: self.project.source(file),
                            at: half.type_name.span.into(),
                        });
                    }
                    let facts = &mut self.defs[d].inlines[&name];
                    match key {
                        "male" => facts.male = type_id,
                        _ => facts.female = type_id,
                    }
                }
            }
        }
    }

    /// Fill each typed connector's port `ConnectorRef.description` from its
    /// resolved connector type. Pass 1 registered the ports without one —
    /// the type wasn't resolved yet.
    pub(super) fn apply_connector_types(&mut self) {
        for d in 0..self.defs.len() {
            let typed: Vec<(&'a str, &'a str, Vec<PortName>)> = self.defs[d]
                .connectors
                .values()
                .filter_map(|facts| {
                    let type_id = facts.type_id?;
                    Some((
                        facts.ast.name.node.as_str(),
                        self.connector_types[type_id].ast.description.node.as_str(),
                        facts
                            .ast
                            .ports
                            .iter()
                            .map(|p| PortName::from(p.name.node.as_str()))
                            .collect(),
                    ))
                })
                .collect();
            for (connector_name, description, port_names) in typed {
                for name in port_names {
                    // A duplicate-named port kept the *first* declaration in
                    // the registry; the name guard skips it if that one
                    // belongs to a different connector.
                    if let Some(port) = self.defs[d].ports.get_mut(&name)
                        && let Some(cref) = &mut port.connector
                        && cref.name == connector_name
                    {
                        cref.description = Some(description);
                    }
                }
            }
        }
    }

    pub(super) fn resolve_endpoints(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            let ast = self.defs[d].ast;
            for member in &ast.members {
                let wires: Vec<&ast::Wire> = match member {
                    Member::Wire(wire) => vec![wire],
                    Member::Cable(cable) => cable.wires().collect(),
                    _ => continue,
                };
                for wire in wires {
                    for ep in &wire.endpoints {
                        if let Some(problem) = self.check_endpoint(d, ep) {
                            problems.push(problem);
                        }
                    }
                }
            }
        }
        self.problems.extend(problems);
    }

    fn check_endpoint(&self, d: DefId, ep: &ast::Endpoint) -> Option<Problem> {
        let file = self.defs[d].file;
        let port = ep.port.node.as_str();
        // Flat namespace: an endpoint resolves against the merged instances
        // and ports of `d`'s whole component, so one fragment's wire may reach
        // an instance or own-port declared in another fragment.
        match &ep.instance {
            None => {
                if self.lookup_port(d, &PortName::from(port)).is_some() {
                    None
                } else {
                    Some(Problem::UnknownPort {
                        port: port.to_string(),
                        on: String::new(),
                        src: self.project.source(file),
                        at: ep.port.span.into(),
                    })
                }
            }
            Some(inst) => {
                let iname = inst.node.as_str();
                let Some(facts) = self.lookup_instance(d, &InstanceName::from(iname)) else {
                    // Not an instance — an inline connector, perhaps. Its
                    // ports are addressable regardless of `pub`.
                    if let Some(inline) = self.lookup_inline(d, &InstanceName::from(iname)) {
                        return if inline.ports.contains_key(&PortName::from(port)) {
                            None
                        } else {
                            Some(Problem::UnknownPort {
                                port: port.to_string(),
                                on: format!(" on inline `{iname}`"),
                                src: self.project.source(file),
                                at: ep.port.span.into(),
                            })
                        };
                    }
                    return Some(Problem::UnknownInstance {
                        name: iname.to_string(),
                        src: self.project.source(file),
                        at: inst.span.into(),
                    });
                };
                let tid = facts.type_id?; // undefined type already reported
                match self.defs[tid].ports.get(&PortName::from(port)) {
                    None => Some(Problem::UnknownPort {
                        port: port.to_string(),
                        on: format!(" on `{}`", self.defs[tid].name),
                        src: self.project.source(file),
                        at: ep.port.span.into(),
                    }),
                    Some(facts) if facts.visibility != ast::Visibility::Public => {
                        Some(Problem::PrivatePort {
                            port: port.to_string(),
                            ty: self.defs[tid].name.to_string(),
                            src: self.project.source(file),
                            at: ep.port.span.into(),
                        })
                    }
                    Some(_) => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dsl::resolve::testkit::{LEAF, codes, has, inline_project, problems};

    #[test]
    fn undefined_type_is_reported() {
        let p = problems(&[("main.wb", "component m { g: ghost; }\n")]);
        assert!(has(&p, "wirebug::undefined_type"), "{:?}", codes(&p));
    }

    #[test]
    fn private_port_access_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { port secret \"S\"; }\ncomponent m { l: leaf; wire red 1 [l.secret, l.secret]; }\n",
        )]);
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_port_on_instance_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { l: leaf; wire red 1 [l.nope, l.a]; }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_instance_in_wire_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { wire red 1 [ghost.a, ghost.b]; }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn inline_wire_endpoint_resolves() {
        let p = inline_project(
            "male: M2; female: F2; port a \"A\" pin 1;",
            "wire red 1 [l.a, ic.a];",
        );
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_inline_property_is_reported() {
        let p = inline_project("plug: M2; port a \"A\" pin 1;", "");
        assert!(
            has(&p, "wirebug::unknown_inline_property"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_inline_half_is_reported() {
        let p = inline_project("male: M2; male: F2; port a \"A\" pin 1;", "");
        assert!(has(&p, "wirebug::duplicate_inline_half"), "{:?}", codes(&p));
    }

    #[test]
    fn undefined_inline_half_type_is_reported() {
        let p = inline_project("male: Ghost2p; port a \"A\" pin 1;", "");
        assert!(
            has(&p, "wirebug::undefined_connector_type"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn unknown_port_on_inline_is_reported() {
        let p = inline_project("port a \"A\" pin 1;", "wire red 1 [l.a, ic.nope];");
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn cable_endpoints_resolve_like_loose_wires() {
        // An unknown port inside a cable wire is caught just like a loose wire.
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; cable c { wire red 1 [a, ghost]; } }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn connector_instance_resolves_local_connector_type() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; } }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn connector_instance_resolves_imported_connector_type() {
        let p = problems(&[
            (
                "main.wb",
                "use ampseal from \"connectors.wb\";\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; } }\n",
            ),
            (
                "connectors.wb",
                "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn unknown_connector_type_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { connector x1: ghost { pub port can_h \"CAN H\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::undefined_connector_type"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn fragment_wire_reaches_an_instance_from_another_fragment() {
        // `traction` wires to `pack`, declared only in `main`'s fragment.
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\n\
                 extend vehicle { inv: leaf \"Inv\"; wire red 1 [pack.a, inv.a]; }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn wire_to_an_instance_in_no_fragment_is_unknown() {
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\n\
                 extend vehicle { inv: leaf \"Inv\"; wire red 1 [ghost.a, inv.a]; }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(has(&p, "wirebug::unknown_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn inline_is_wireable_from_another_fragment() {
        let p = problems(&[
            (
                "main.wb",
                "use m from \"loom.wb\";\nuse leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\n",
            ),
            (
                "loom.wb",
                "extend m { inline ic { port a \"A\" pin 1; } wire red 1 [l.a, ic.a]; }\n",
            ),
            ("leaf.wb", "component leaf { pub port a \"A\"; }\n"),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }
}
