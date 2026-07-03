//! Pass 1 of resolution: walk every definition's AST and register its
//! facts — flattened ports (connectors are grouping metadata, not a
//! namespace), connector instances, child instances, inline connectors —
//! reporting the intra-definition duplicates as it goes.

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::dsl::ast::{self, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{ConnectorName, InstanceName, Pin, PortName};
use crate::dsl::span::{FileId, Span};

use super::{
    ConnectorInstFacts, ConnectorRef, ConnectorTypeId, ConnectorTypeInfo, DefId, DefInfo,
    InlineFacts, InstFacts, PortFacts, Resolver,
};

impl<'a> Resolver<'a> {
    /// Register a definition and its nested definitions, flattening ports
    /// and collecting instances. Returns the new [`DefId`].
    pub(super) fn register(
        &mut self,
        def: &'a ast::Definition,
        file: FileId,
        parent: Option<DefId>,
    ) -> DefId {
        let id = self.defs.len();
        // `extend` only splits a top-level component; nested fragments make no
        // sense (a nested definition is private to its parent).
        if def.kind == ast::DefKind::Extend && parent.is_some() {
            self.problems.push(Problem::NestedExtend {
                src: self.project.source(file),
                at: def.name.span.into(),
            });
        }
        self.defs.push(DefInfo {
            name: def.name.node.as_str(),
            file,
            kind: def.kind,
            ast: def,
            parent,
            ports: IndexMap::new(),
            connectors: IndexMap::new(),
            instances: IndexMap::new(),
            inlines: IndexMap::new(),
            nested: Vec::new(),
        });

        let mut ports: IndexMap<PortName, PortFacts<'a>> = IndexMap::new();
        let mut connectors: IndexMap<ConnectorName, ConnectorInstFacts<'a>> = IndexMap::new();
        let mut instances: IndexMap<InstanceName, InstFacts<'a>> = IndexMap::new();
        let mut inlines: IndexMap<InstanceName, InlineFacts<'a>> = IndexMap::new();
        let mut nested = Vec::new();
        let mut connector_index = 0;
        let mut connector_names: HashMap<&str, Span> = HashMap::new();
        let mut cable_names: HashMap<&str, Span> = HashMap::new();

        for member in &def.members {
            match member {
                Member::Port(port) => self.add_port(&mut ports, port, None, file),
                Member::Connector(conn) => {
                    let name = conn.name.node.as_str();
                    if let Some(&first) = connector_names.get(name) {
                        self.problems.push(Problem::DuplicateConnectorName {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: conn.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        connector_names.insert(name, conn.name.span);
                    }
                    let cref = ConnectorRef {
                        name,
                        description: conn.description.as_ref().map(|d| d.node.as_str()),
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(&mut ports, port, Some(cref), file);
                    }
                    self.check_duplicate_pins(name, &conn.ports, file);
                }
                Member::ConnectorInstance(conn) => {
                    let name = ConnectorName::from(conn.name.node.as_str());
                    if let Some(first) = connector_names.get(conn.name.node.as_str()) {
                        self.problems.push(Problem::DuplicateConnectorName {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: conn.name.span.into(),
                            first: (*first).into(),
                        });
                    } else {
                        connector_names.insert(conn.name.node.as_str(), conn.name.span);
                        connectors.insert(
                            name,
                            ConnectorInstFacts {
                                ast: conn,
                                type_id: None,
                                index: connector_index,
                            },
                        );
                    }
                    // Same flat port declarations as an inline connector; the
                    // description comes from the connector type, resolved
                    // later — `apply_connector_types` fills it in.
                    let cref = ConnectorRef {
                        name: conn.name.node.as_str(),
                        description: None,
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(&mut ports, port, Some(cref), file);
                    }
                    self.check_duplicate_pins(conn.name.node.as_str(), &conn.ports, file);
                }
                Member::Instance(inst) => {
                    let name = InstanceName::from(inst.name.node.as_str());
                    // Inlines share the instance namespace (wire endpoints
                    // address both the same way).
                    let first = instances
                        .get(&name)
                        .map(|f| f.ast.name.span)
                        .or_else(|| inlines.get(&name).map(|f| f.ast.name.span));
                    if let Some(first) = first {
                        self.problems.push(Problem::DuplicateInstance {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: inst.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        instances.insert(
                            name,
                            InstFacts {
                                ast: inst,
                                type_id: None,
                            },
                        );
                    }
                }
                Member::Inline(inline) => {
                    let name = InstanceName::from(inline.name.node.as_str());
                    let first = instances
                        .get(&name)
                        .map(|f| f.ast.name.span)
                        .or_else(|| inlines.get(&name).map(|f| f.ast.name.span));
                    if let Some(first) = first {
                        self.problems.push(Problem::DuplicateInstance {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: inline.name.span.into(),
                            first: first.into(),
                        });
                        continue;
                    }
                    // The inline's own port set — a fresh map, so its port
                    // names only collide within the inline. `pub` is
                    // meaningless here: the ports are only addressable
                    // within the defining component.
                    let mut inline_ports: IndexMap<PortName, PortFacts<'a>> = IndexMap::new();
                    for port in &inline.ports {
                        if port.visibility == ast::Visibility::Public {
                            self.problems.push(Problem::InlinePubPort {
                                src: self.project.source(file),
                                at: port.name.span.into(),
                            });
                        }
                        self.add_port(&mut inline_ports, port, None, file);
                    }
                    self.check_duplicate_pins(inline.name.node.as_str(), &inline.ports, file);
                    inlines.insert(
                        name,
                        InlineFacts {
                            ast: inline,
                            ports: inline_ports,
                            male: None,
                            female: None,
                        },
                    );
                }
                Member::Wire(_) => {} // endpoints resolved in pass 2
                Member::Cable(cable) => {
                    // endpoints resolved in pass 2; here, guard the designator.
                    let n = cable.name.node.as_str();
                    if let Some(&first) = cable_names.get(n) {
                        self.problems.push(Problem::DuplicateCableName {
                            name: n.to_string(),
                            src: self.project.source(file),
                            at: cable.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        cable_names.insert(n, cable.name.span);
                    }
                }
                Member::Definition(child) => {
                    let child_id = self.register(child, file, Some(id));
                    nested.push(child_id);
                }
            }
        }

        self.defs[id].ports = ports;
        self.defs[id].connectors = connectors;
        self.defs[id].instances = instances;
        self.defs[id].inlines = inlines;
        self.defs[id].nested = nested;
        id
    }

    pub(super) fn register_connector_type(
        &mut self,
        connector_type: &'a ast::ConnectorType,
        file: FileId,
    ) -> ConnectorTypeId {
        let id = self.connector_types.len();
        self.connector_types.push(ConnectorTypeInfo {
            name: connector_type.name.node.as_str(),
            file,
            ast: connector_type,
        });
        id
    }

    /// One cavity, one port: flag a pin number claimed twice within a single
    /// connector (inline or typed), including twice in one `pins [..]` list.
    fn check_duplicate_pins(&mut self, connector: &str, ports: &[ast::Port], file: FileId) {
        let mut pins: HashMap<u32, Span> = HashMap::new();
        for port in ports {
            for pin in &port.pins {
                if let Some(&first) = pins.get(&pin.node) {
                    self.problems.push(Problem::DuplicateConnectorPin {
                        pin: pin.node,
                        connector: connector.to_string(),
                        src: self.project.source(file),
                        at: pin.span.into(),
                        first: first.into(),
                    });
                } else {
                    pins.insert(pin.node, pin.span);
                }
            }
        }
    }

    fn add_port(
        &mut self,
        ports: &mut IndexMap<PortName, PortFacts<'a>>,
        port: &'a ast::Port,
        connector: Option<ConnectorRef<'a>>,
        file: FileId,
    ) {
        let name = PortName::from(port.name.node.as_str());
        if let Some(first) = ports.get(&name) {
            self.problems.push(Problem::DuplicatePort {
                name: name.to_string(),
                src: self.project.source(file),
                at: port.name.span.into(),
                first: first.span.into(),
            });
            return;
        }
        ports.insert(
            name,
            PortFacts {
                visibility: port.visibility,
                label: port.label.node.as_str(),
                pins: port.pins.iter().map(|p| Pin(p.node)).collect(),
                connector,
                span: port.name.span,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::dsl::resolve::testkit::{codes, has, inline_project, problems};

    #[test]
    fn duplicate_instance_name_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { x: leaf; x: leaf; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_port_across_connectors_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; connector c \"C\" { pub port a \"A2\" pin 1; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn inline_shares_the_instance_namespace() {
        let p = inline_project("port a \"A\" pin 1;", "ic: leaf;");
        assert!(has(&p, "wirebug::duplicate_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn inline_duplicate_port_is_reported() {
        let p = inline_project("port a \"A\" pin 1; port a \"A2\" pin 2;", "");
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn inline_duplicate_pin_is_reported() {
        let p = inline_project("port a \"A\" pin 1; port b \"B\" pin 1;", "");
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn inline_pub_port_warns() {
        let p = inline_project("pub port a \"A\" pin 1;", "");
        assert!(has(&p, "wirebug::inline_pub_port"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_connector_name_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { connector x \"A\" { pub port a \"A\" pin 1; } connector x \"B\" { pub port b \"B\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_name"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_cable_name_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; cable c { wire red 1 [a, b]; } cable c { wire red 1 [a, b]; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_cable_name"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_connector_pin_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; pub port can_l \"CAN L\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_pin_errors_on_inline_connectors_too() {
        let p = problems(&[(
            "main.wb",
            "component m { connector x1 \"X 2p\" { pub port a \"A\" pin 1; pub port b \"B\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn one_port_can_occupy_multiple_pins_on_one_connector() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pub port gnd \"GND\" pins [1, 2]; } }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn typed_connector_ports_are_component_ports() {
        // A port declared inside a typed connector is an ordinary flat
        // component port: wires reach it, and its name collides with a
        // bare port of the same name.
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port a \"A\"; connector x1: ampseal { pub port b \"B\" pin 1; } wire red 1 [a, b]; }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));

        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port a \"A\"; connector x1: ampseal { pub port a \"A\" pin 1; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn nested_extend_is_rejected() {
        let p = problems(&[("main.wb", "component outer { extend inner { } }\n")]);
        assert!(has(&p, "wirebug::nested_extend"), "{:?}", codes(&p));
    }
}
