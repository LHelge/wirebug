//! Elaboration: turn the resolved registry of definitions and instances
//! into a flat-map, hierarchical [`Design`] of concrete instances.
//!
//! Walk from the root definition (main.wb's sole top-level component),
//! stamp one IR instance per placement with a dotted path, materialize
//! its ports, rewrite its wires against the local scope, and recurse into
//! its children. Definitions vanish here; only instances flow to the IR.
//! A type stack guards against containment cycles.

use crate::dsl::ast::{self, CablePropertyValue, ConnectorPropertyValue, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{
    CableMeta, CableName, Connector, ConnectorCavity, ConnectorFaceLayout, ConnectorGridLayout,
    ConnectorLayout, ConnectorName, ConnectorPin, ConnectorRef, ConnectorTypeName, Design,
    EnclosurePort, Half, Include, InlineHalfMeta, InlineMeta, Instance, InstanceName, InstancePath,
    Port, PortName, Side, TypeName, View, Visibility, Wire, WireColor, WireEnd,
};
use crate::dsl::resolve::{DefId, Resolved};

use indexmap::IndexMap;

/// Elaborate the resolved registry into a [`Design`]. Returns `None` only
/// when there is no usable root; partial designs are still returned
/// alongside problems (e.g. a containment cycle skips the cyclic node).
pub fn elaborate(resolved: &Resolved) -> (Option<Design>, Vec<Problem>) {
    let mut e = Elaborator {
        resolved,
        instances: IndexMap::new(),
        problems: Vec::new(),
    };

    let Some(root_id) = resolved.root else {
        e.problems.push(Problem::NoRoot);
        return (None, e.problems);
    };

    let root_path = InstancePath::root(InstanceName::from(resolved.defs[root_id].name));
    e.stamp(root_id, &root_path, None, &mut Vec::new());

    let views = e.elaborate_views();
    let Elaborator {
        instances,
        problems,
        ..
    } = e;

    let design = Design {
        root: root_path,
        instances,
        views,
        manifest: resolved.project.manifest.clone(),
    };
    (Some(design), problems)
}

struct Elaborator<'a> {
    resolved: &'a Resolved<'a>,
    instances: IndexMap<InstancePath, Instance>,
    problems: Vec<Problem>,
}

impl Elaborator<'_> {
    /// Stamp the instance at `path` (of type `def`) and recurse into its
    /// children. `label` is the instantiating site's label (none for the
    /// root). `type_stack` holds the def ids currently being elaborated,
    /// for cycle detection.
    fn stamp(
        &mut self,
        def: DefId,
        path: &InstancePath,
        label: Option<String>,
        type_stack: &mut Vec<DefId>,
    ) {
        if type_stack.contains(&def) {
            let info = &self.resolved.defs[def];
            let cycle = type_stack
                .iter()
                .chain(std::iter::once(&def))
                .map(|&d| self.resolved.defs[d].name)
                .collect::<Vec<_>>()
                .join(" -> ");
            self.problems.push(Problem::ContainmentCycle {
                name: info.name.to_string(),
                cycle,
                src: self.resolved_source(def),
                at: info.ast.name.span.into(),
            });
            return;
        }
        type_stack.push(def);

        // A merged component is authored as several fragments (a `component`
        // plus its `extend`s); they share one flat namespace, so stamp the
        // union of every fragment's ports, wires, connectors, and children.
        // `fragments` is canonical-first and just `[def]` for an unmerged type.
        let fragments = self.resolved.fragments(def);

        let mut ports: IndexMap<PortName, Port> = IndexMap::new();
        for &frag in &fragments {
            for (name, facts) in &self.resolved.defs[frag].ports {
                ports.entry(name.clone()).or_insert_with(|| Port {
                    name: name.clone(),
                    label: facts.label.to_string(),
                    visibility: match facts.visibility {
                        ast::Visibility::Public => Visibility::Public,
                        ast::Visibility::Private => Visibility::Private,
                    },
                    connector: facts.connector.as_ref().map(|c| ConnectorRef {
                        name: ConnectorName::from(c.name),
                        description: c.description.map(str::to_string),
                        index: c.index,
                    }),
                    pins: facts.pins.clone(),
                });
            }
        }

        // Loose wires keep `cable: None`; a cable's conductors are tagged with
        // its designator and its metadata recorded separately. Property and
        // arity problems are reported once per def in `validate`, so this pass
        // is best-effort: it takes well-typed values and ignores the rest.
        let mut wires = Vec::new();
        let mut cables: IndexMap<CableName, CableMeta> = IndexMap::new();
        let mut connectors: IndexMap<ConnectorName, Connector> = IndexMap::new();
        for &frag in &fragments {
            let info = &self.resolved.defs[frag];
            for m in &info.ast.members {
                match m {
                    Member::Wire(w) => wires.push(rewrite_wire(w, None, None)),
                    Member::Cable(c) => {
                        let name = CableName::from(c.name.node.as_str());
                        cables.insert(name.clone(), cable_meta(c));
                        let mut group = 0u32;
                        for m in &c.members {
                            match m {
                                ast::CableMember::Wire(w) => {
                                    wires.push(rewrite_wire(w, Some(name.clone()), None));
                                }
                                ast::CableMember::Twisted(t) => {
                                    for w in t.wires.iter() {
                                        wires.push(rewrite_wire(
                                            w,
                                            Some(name.clone()),
                                            Some(group),
                                        ));
                                    }
                                    group += 1;
                                }
                            }
                        }
                    }
                    Member::Connector(conn) => {
                        let name = ConnectorName::from(conn.name.node.as_str());
                        connectors.insert(name.clone(), inline_connector(name, conn));
                    }
                    Member::ConnectorInstance(conn) => {
                        let name = ConnectorName::from(conn.name.node.as_str());
                        let Some(facts) = info.connectors.get(&name) else {
                            continue;
                        };
                        let Some(type_id) = facts.type_id else {
                            continue;
                        };
                        let connector_type = self.resolved.connector_types[type_id].ast;
                        connectors
                            .insert(name.clone(), typed_connector(name, conn, connector_type));
                    }
                    _ => {}
                }
            }
        }

        // Resolve child placements (skip instances whose type didn't
        // resolve — that error is already reported).
        let mut children = IndexMap::new();
        let mut child_jobs: Vec<(InstanceName, DefId, Option<String>)> = Vec::new();
        for &frag in &fragments {
            for (name, inst) in &self.resolved.defs[frag].instances {
                let Some(tid) = inst.type_id else { continue };
                let label = inst.ast.label.as_ref().map(|l| l.node.clone());
                child_jobs.push((name.clone(), tid, label));
            }
        }
        for (name, _, _) in &child_jobs {
            children.insert(name.clone(), path.child(name.clone()));
        }

        // Inline (mid-harness) connectors become synthetic child instances:
        // one node per mated pair, its shared port set addressable like any
        // child's, the housing halves' part metadata in `Instance.inline`.
        let mut inline_jobs: Vec<Instance> = Vec::new();
        for &frag in &fragments {
            for (name, facts) in &self.resolved.defs[frag].inlines {
                if children.contains_key(name) {
                    continue; // duplicate name, already reported
                }
                let child_path = path.child(name.clone());
                children.insert(name.clone(), child_path.clone());
                inline_jobs.push(self.inline_instance(child_path, facts));
            }
        }

        self.instances.insert(
            path.clone(),
            Instance {
                path: path.clone(),
                type_name: TypeName::from(self.resolved.defs[def].name),
                label,
                ports,
                children,
                wires,
                cables,
                connectors,
                inline: None,
            },
        );

        for (name, tid, label) in child_jobs {
            let child_path = path.child(name);
            self.stamp(tid, &child_path, label, type_stack);
        }
        for instance in inline_jobs {
            self.instances.insert(instance.path.clone(), instance);
        }

        type_stack.pop();
    }

    /// Materialize an inline connector as a synthetic child [`Instance`]:
    /// public ports (they are only addressable within the defining
    /// component anyway), one designator-named connector carrying the pin
    /// bindings, and the halves' connector-type metadata in `inline`.
    fn inline_instance(
        &self,
        path: InstancePath,
        facts: &crate::dsl::resolve::InlineFacts,
    ) -> Instance {
        let inline = facts.ast;
        let designator = ConnectorName::from(inline.name.node.as_str());
        let cref = ConnectorRef {
            name: designator.clone(),
            description: None,
            index: 0,
        };
        let ports: IndexMap<PortName, Port> = facts
            .ports
            .iter()
            .map(|(name, pf)| {
                (
                    name.clone(),
                    Port {
                        name: name.clone(),
                        label: pf.label.to_string(),
                        visibility: Visibility::Public,
                        connector: Some(cref.clone()),
                        pins: pf.pins.clone(),
                    },
                )
            })
            .collect();
        let connector = Connector {
            name: designator.clone(),
            type_name: None,
            description: None,
            properties: IndexMap::new(),
            layout: None,
            pins: inline
                .ports
                .iter()
                .flat_map(|port| {
                    port.pins.iter().map(|pin| ConnectorPin {
                        pin: crate::dsl::ir::Pin(pin.node),
                        port: PortName::from(port.name.node.as_str()),
                    })
                })
                .collect(),
        };
        let half_meta = |type_id: Option<usize>| {
            let connector_type = self.resolved.connector_types[type_id?].ast;
            Some(InlineHalfMeta {
                type_name: ConnectorTypeName::from(connector_type.name.node.as_str()),
                description: connector_type.description.node.clone(),
                properties: connector_type_properties(connector_type),
                layout: connector_type_layout(connector_type),
            })
        };
        Instance {
            path,
            type_name: TypeName::from(inline.name.node.as_str()),
            label: inline.label.as_ref().map(|l| l.node.clone()),
            ports,
            children: IndexMap::new(),
            wires: Vec::new(),
            cables: IndexMap::new(),
            connectors: IndexMap::from([(designator, connector)]),
            inline: Some(InlineMeta {
                male: half_meta(facts.male),
                female: half_meta(facts.female),
            }),
        }
    }

    fn elaborate_views(&self) -> Vec<View> {
        self.resolved
            .views
            .iter()
            .filter_map(|binding| {
                let subject = binding.subject?;
                let v = binding.ast;
                let kind = crate::dsl::ir::ViewKind::from(v.kind.node.as_str());
                let is_harness = kind.is_harness();
                Some(View {
                    kind,
                    title: v.title.node.clone(),
                    grid: v.grid.as_ref().map(|g| g.node),
                    subject: TypeName::from(self.resolved.defs[subject].name),
                    has_enclosure: v.has_enclosure,
                    // Anchors whose shape resolve already rejected drop here.
                    enclosure: v
                        .enclosure
                        .iter()
                        .filter_map(elaborate_enclosure_port)
                        .collect(),
                    includes: v
                        .includes
                        .iter()
                        .map(|inc| {
                            let instance = InstanceName::from(inc.instance.node.as_str());
                            // An inline-connector include is normalized: the
                            // authored `male`/`female` segment moves to
                            // `half`, and `connector` becomes the inline's
                            // designator — which is what the harness layout
                            // keys nodes and endpoints by.
                            let is_inline =
                                is_harness
                                    && self.resolved.fragments(subject).into_iter().any(|f| {
                                        self.resolved.defs[f].inlines.contains_key(&instance)
                                    });
                            let (connector, half) = if is_inline {
                                (
                                    Some(ConnectorName::from(instance.as_str())),
                                    inc.connector
                                        .as_ref()
                                        .and_then(|c| c.node.as_str().parse::<Half>().ok()),
                                )
                            } else {
                                (
                                    inc.connector
                                        .as_ref()
                                        .map(|c| ConnectorName::from(c.node.as_str())),
                                    None,
                                )
                            };
                            Include {
                                instance,
                                connector,
                                half,
                                x: inc.x.node,
                                y: inc.y.node,
                                // Sides that failed to parse were already reported
                                // by resolve; drop them here.
                                ports: inc
                                    .ports
                                    .iter()
                                    .filter_map(|p| {
                                        let side = p.side.node.as_str().parse::<Side>().ok()?;
                                        Some((PortName::from(p.port.node.as_str()), side))
                                    })
                                    .collect(),
                            }
                        })
                        .collect(),
                    texts: v
                        .texts
                        .iter()
                        .map(|text| crate::dsl::ir::TextBox {
                            name: text.name.node.as_str().to_string(),
                            x: text.x.node,
                            y: text.y.node,
                            label: text.label.node.clone(),
                        })
                        .collect(),
                })
            })
            .collect()
    }

    fn resolved_source(&self, def: DefId) -> miette::NamedSource<String> {
        self.resolved.project.source(self.resolved.defs[def].file)
    }
}

/// Resolve an enclosure port's `(x, y)` anchor to a `(side, coord)` pair:
/// the side names the edge, the coordinate positions the port along the free
/// axis. Returns `None` for an anchor whose shape resolve already rejected.
fn elaborate_enclosure_port(ep: &ast::EnclosurePort) -> Option<EnclosurePort> {
    use ast::Anchor::{Coord, Edge};
    let (side, coord) = match (&ep.x, &ep.y) {
        (Edge(s), Coord(c)) => (s.node.as_str().parse::<Side>().ok()?, c.node),
        (Coord(c), Edge(s)) => (s.node.as_str().parse::<Side>().ok()?, c.node),
        _ => return None,
    };
    // West/east belong in the x slot, north/south in the y slot; a mismatch
    // was reported by resolve, so drop it rather than place it on the wrong edge.
    let x_slot = matches!(ep.x, Edge(_));
    let fits = match side {
        Side::West | Side::East => x_slot,
        Side::North | Side::South => !x_slot,
    };
    if !fits {
        return None;
    }
    Some(EnclosurePort {
        port: PortName::from(ep.port.node.as_str()),
        side,
        coord,
    })
}

fn rewrite_wire(w: &ast::Wire, cable: Option<CableName>, twisted_group: Option<u32>) -> Wire {
    Wire {
        color: WireColor::new(
            w.color.node.as_str(),
            w.tracer.as_ref().map(|t| t.node.as_str()),
        ),
        gauge: w.gauge.node,
        label: w.label.as_ref().map(|l| l.node.clone()),
        endpoints: w
            .endpoints
            .iter()
            .map(|ep| match &ep.instance {
                None => WireEnd::Own(PortName::from(ep.port.node.as_str())),
                Some(inst) => WireEnd::Child {
                    instance: InstanceName::from(inst.node.as_str()),
                    port: PortName::from(ep.port.node.as_str()),
                },
            })
            .collect(),
        cable,
        twisted_group,
    }
}

/// Map a cable's faithful AST properties to typed [`CableMeta`]. Unknown keys
/// and wrong value types are reported in `validate`; here they are dropped.
fn cable_meta(c: &ast::Cable) -> CableMeta {
    let mut meta = CableMeta {
        label: c.label.as_ref().map(|l| l.node.clone()),
        r#type: None,
        length: None,
    };
    for p in &c.properties {
        match (p.key.node.as_str(), &p.value) {
            ("type", CablePropertyValue::Str(s)) => meta.r#type = Some(s.node.clone()),
            ("length", CablePropertyValue::Number(n)) => meta.length = Some(n.node),
            _ => {}
        }
    }
    meta
}

fn inline_connector(name: ConnectorName, conn: &ast::Connector) -> Connector {
    Connector {
        name,
        type_name: None,
        description: conn.description.as_ref().map(|d| d.node.clone()),
        properties: IndexMap::new(),
        layout: None,
        pins: conn
            .ports
            .iter()
            .flat_map(|port| {
                port.pins.iter().map(|pin| ConnectorPin {
                    pin: crate::dsl::ir::Pin(pin.node),
                    port: PortName::from(port.name.node.as_str()),
                })
            })
            .collect(),
    }
}

fn typed_connector(
    name: ConnectorName,
    conn: &ast::ConnectorInstance,
    connector_type: &ast::ConnectorType,
) -> Connector {
    Connector {
        name,
        type_name: Some(ConnectorTypeName::from(connector_type.name.node.as_str())),
        description: Some(connector_type.description.node.clone()),
        properties: connector_type_properties(connector_type),
        layout: connector_type_layout(connector_type),
        pins: conn
            .ports
            .iter()
            .flat_map(|port| {
                port.pins.iter().map(|pin| ConnectorPin {
                    pin: crate::dsl::ir::Pin(pin.node),
                    port: PortName::from(port.name.node.as_str()),
                })
            })
            .collect(),
    }
}

/// Map a connector type's faithful AST properties to the typed IR map.
fn connector_type_properties(
    connector_type: &ast::ConnectorType,
) -> IndexMap<String, crate::dsl::ir::ConnectorPropertyValue> {
    connector_type
        .properties
        .iter()
        .map(|p| {
            (
                p.key.node.as_str().to_string(),
                match &p.value {
                    ConnectorPropertyValue::Str(s) => {
                        crate::dsl::ir::ConnectorPropertyValue::Str(s.node.clone())
                    }
                    ConnectorPropertyValue::Number(n) => {
                        crate::dsl::ir::ConnectorPropertyValue::Number(n.node)
                    }
                },
            )
        })
        .collect()
}

fn connector_type_layout(connector_type: &ast::ConnectorType) -> Option<ConnectorLayout> {
    match connector_type.layout.as_ref()? {
        ast::ConnectorLayout::Grid(layout) => Some(ConnectorLayout::Grid(ConnectorGridLayout {
            rows: layout.rows.node,
            cols: layout.cols.node,
            numbering: layout
                .numbering
                .as_ref()
                .map(|n| n.node.as_str().to_string()),
        })),
        ast::ConnectorLayout::Face(layout) => Some(ConnectorLayout::Face(ConnectorFaceLayout {
            cavities: layout
                .cavities
                .iter()
                .map(|cavity| ConnectorCavity {
                    pin: crate::dsl::ir::Pin(cavity.pin.node),
                    x: cavity.x.node,
                    y: cavity.y.node,
                    size: cavity.size.as_ref().map(|s| s.node.as_str().to_string()),
                })
                .collect(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::project::load;
    use crate::dsl::resolve::resolve;

    fn elaborate_files(files: &[(&str, &str)]) -> (Option<Design>, Vec<Problem>) {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let (project, _) = load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = resolve(&project);
        let (design, problems) = elaborate(&resolved);
        (design, problems)
    }

    #[test]
    fn elaborates_a_multi_file_design_tree() {
        let main = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/main.wb"
        ));
        let (project, _) = load(&main);
        let project = project.expect("loads");
        let resolved = resolve(&project);
        let (design, problems) = elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        let design = design.expect("a design");

        assert_eq!(design.root.to_string(), "Vehicle");

        // Deep path materialized down through the hierarchy.
        let pack = InstancePath::root(InstanceName::from("Vehicle"))
            .child(InstanceName::from("pack"))
            .child(InstanceName::from("pack"));
        let pack_inst = design.get(&pack).expect("Vehicle.pack.pack exists");
        assert_eq!(pack_inst.type_name.as_str(), "CellPack");
        assert!(pack_inst.ports.contains_key(&PortName::from("hv_pos")));

        // Imported child instances are stamped at the root level.
        let battery =
            InstancePath::root(InstanceName::from("Vehicle")).child(InstanceName::from("pack"));
        assert_eq!(design.get(&battery).unwrap().type_name.as_str(), "Battery");

        // The root owns two point-to-point HV wires between the included
        // battery and inverter.
        let vehicle = design.get(&design.root).unwrap();
        assert!(
            vehicle
                .wires
                .iter()
                .all(|w| w.gauge == 50.0 && w.endpoints.len() == 2),
            "the fixture root uses only two-endpoint HV wires"
        );

        // Views came through, bound to their type.
        assert!(design.views.iter().any(|v| v.subject.as_str() == "Vehicle"));
    }

    #[test]
    fn extend_fragments_merge_into_one_instance() {
        let (design, problems) = elaborate_files(&[
            (
                "main.wb",
                "use vehicle from \"frag.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "frag.wb",
                "use leaf from \"leaf.wb\";\n\
                 extend vehicle { inv: leaf \"Inv\"; wire red 1 [pack.a, inv.a]; }\n",
            ),
            ("leaf.wb", "component leaf { pub port a \"A\"; }\n"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        assert_eq!(design.root.to_string(), "vehicle");

        // Children from both fragments are stamped under the one merged root.
        let root = InstancePath::root(InstanceName::from("vehicle"));
        let pack = root.clone().child(InstanceName::from("pack"));
        let inv = root.clone().child(InstanceName::from("inv"));
        assert_eq!(design.get(&pack).unwrap().type_name.as_str(), "leaf");
        assert_eq!(design.get(&inv).unwrap().type_name.as_str(), "leaf");

        // The fragment's wire — written in frag.wb but reaching `pack` from
        // main.wb — lands on the merged root as a `Child`→`Child` net.
        let vehicle = design.get(&design.root).unwrap();
        assert_eq!(vehicle.wires.len(), 1);
        assert!(
            vehicle.wires[0]
                .endpoints
                .iter()
                .all(|e| matches!(e, WireEnd::Child { .. })),
            "cross-fragment endpoints rewrite to children: {:?}",
            vehicle.wires[0].endpoints
        );
    }

    #[test]
    fn cable_tags_its_wires_and_records_metadata() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "component m {
                pub port a \"A\"; pub port b \"B\"; pub port x \"X\"; pub port y \"Y\";
                wire black 1 [a, b];
                cable feed \"Power feed\" {
                    type: \"2-core\";
                    length: 0.8;
                    wire red 1 [x, y];
                }
            }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let m = design.unwrap();
        let root = m.get(&m.root).unwrap();

        // The loose wire is untagged; the cable's conductor carries its name.
        let loose = root
            .wires
            .iter()
            .find(|w| w.color.css() == "black")
            .unwrap();
        assert!(loose.cable.is_none());
        let conductor = root.wires.iter().find(|w| w.color.css() == "red").unwrap();
        assert_eq!(conductor.cable.as_ref().map(|c| c.as_str()), Some("feed"));

        // The metadata is recorded once, typed, keyed by designator.
        let meta = root
            .cables
            .get(&CableName::from("feed"))
            .expect("cable meta");
        assert_eq!(meta.label.as_deref(), Some("Power feed"));
        assert_eq!(meta.r#type.as_deref(), Some("2-core"));
        assert_eq!(meta.length, Some(0.8));
    }

    #[test]
    fn twisted_groups_number_their_wires_per_cable() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "component m {
                pub port a \"A\"; pub port b \"B\";
                pub port c \"C\"; pub port d \"D\";
                pub port e \"E\"; pub port f \"F\";
                cable loom {
                    wire red 1 [a, b];
                    twisted {
                        wire white/blue 0.5 [c, d];
                        wire white/red 0.5 [e, f];
                    }
                }
            }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let m = design.unwrap();
        let root = m.get(&m.root).unwrap();

        // The plain conductor has no group; the pair shares group 0.
        let groups: Vec<Option<u32>> = root.wires.iter().map(|w| w.twisted_group).collect();
        assert_eq!(groups, [None, Some(0), Some(0)]);
        // Loose wires (outside any cable) are never grouped.
        assert!(root.wires.iter().all(|w| w.cable.is_some()));
    }

    #[test]
    fn connector_instances_materialize_type_metadata_and_pin_bindings() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" {
                part: \"TE 776164-1\";
                cavities: 35;
                layout grid {
                    rows: 1;
                    cols: 2;
                    numbering: row_major;
                }
            }
            component m {
                connector x1: ampseal {
                    pub port can_h \"CAN H\" pin 1;
                    pub port can_l \"CAN L\" pin 2;
                }
            }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        let root = design.get(&design.root).unwrap();
        let connector = root
            .connectors
            .get(&ConnectorName::from("x1"))
            .expect("connector");

        assert_eq!(
            connector.type_name.as_ref().map(|n| n.as_str()),
            Some("ampseal")
        );
        assert_eq!(connector.description.as_deref(), Some("AMPSEAL"));
        assert_eq!(
            connector.properties.get("part"),
            Some(&crate::dsl::ir::ConnectorPropertyValue::Str(
                "TE 776164-1".to_string()
            ))
        );
        assert_eq!(
            connector.layout,
            Some(ConnectorLayout::Grid(ConnectorGridLayout {
                rows: 1,
                cols: 2,
                numbering: Some("row_major".to_string()),
            }))
        );
        assert_eq!(
            connector.pins,
            vec![
                ConnectorPin {
                    pin: crate::dsl::ir::Pin(1),
                    port: PortName::from("can_h")
                },
                ConnectorPin {
                    pin: crate::dsl::ir::Pin(2),
                    port: PortName::from("can_l")
                },
            ]
        );
    }

    #[test]
    fn inline_connectors_materialize_for_compatibility() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "component m {
                connector x1 \"Legacy 2p\" {
                    pub port a \"A\" pin 1;
                    pub port b \"B\" pin 2;
                }
            }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        let root = design.get(&design.root).unwrap();
        let connector = root
            .connectors
            .get(&ConnectorName::from("x1"))
            .expect("connector");

        assert!(connector.type_name.is_none());
        assert_eq!(connector.description.as_deref(), Some("Legacy 2p"));
        assert_eq!(connector.pins.len(), 2);
    }

    #[test]
    fn inline_member_elaborates_as_a_synthetic_child() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "connector_type m4 \"DT04-4P\" {
                part: \"DT04-4P\";
                layout grid { rows: 2; cols: 2; numbering: row_major; }
            }
            connector_type f4 \"DT06-4S\" { part: \"DT06-4S\"; }
            component top {
                pub port a \"A\"; pub port b \"B\";
                inline ic \"Pedal branch\" {
                    male: m4;
                    female: f4;
                    port sig \"SIG\" pin 1;
                    port gnd \"GND\" pin 2;
                }
                cable left { wire red 1 [a, ic.sig]; }
                cable right { wire red 1 [ic.sig, b]; }
            }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        let root = design.get(&design.root).unwrap();

        // The inline is a child like any instance, addressable by path.
        let ic_path = design.root.clone().child(InstanceName::from("ic"));
        assert_eq!(root.children.get(&InstanceName::from("ic")), Some(&ic_path));
        let ic = design.get(&ic_path).expect("synthetic inline instance");
        assert_eq!(ic.label.as_deref(), Some("Pedal branch"));

        // Its ports are public and grouped under the designator-named
        // connector, pins bound from the port clauses.
        let sig = ic.ports.get(&PortName::from("sig")).expect("sig port");
        assert_eq!(sig.visibility, Visibility::Public);
        assert_eq!(sig.connector.as_ref().map(|c| c.name.as_str()), Some("ic"));
        let conn = ic.connectors.get(&ConnectorName::from("ic")).unwrap();
        assert_eq!(conn.pins.len(), 2);

        // The halves carry their connector types' metadata.
        let meta = ic.inline.as_ref().expect("inline meta");
        let male = meta.male.as_ref().expect("male half");
        assert_eq!(male.description, "DT04-4P");
        assert_eq!(
            male.properties.get("part"),
            Some(&crate::dsl::ir::ConnectorPropertyValue::Str(
                "DT04-4P".to_string()
            ))
        );
        assert!(male.layout.is_some());
        let female = meta.female.as_ref().expect("female half");
        assert_eq!(female.type_name.as_str(), "f4");
        assert!(female.layout.is_none());

        // Both cables' conductors land on the inline as ordinary
        // `WireEnd::Child` endpoints sharing the pin.
        let on_ic = |w: &Wire| {
            w.endpoints.iter().any(
                |e| matches!(e, WireEnd::Child { instance, port } if instance.as_str() == "ic" && port.as_str() == "sig"),
            )
        };
        assert_eq!(root.wires.iter().filter(|w| on_ic(w)).count(), 2);
    }

    #[test]
    fn half_less_inline_elaborates_with_empty_meta() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "component top { inline ic { port a \"A\" pin 1; } }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        let ic_path = design.root.clone().child(InstanceName::from("ic"));
        let ic = design.get(&ic_path).unwrap();
        let meta = ic.inline.as_ref().expect("inline meta");
        assert!(meta.male.is_none() && meta.female.is_none());
    }

    #[test]
    fn inline_include_is_normalized_to_designator_plus_half() {
        let (design, problems) = elaborate_files(&[(
            "main.wb",
            "connector_type f4 \"DT06-4S\" { }
            component top {
                pub port a \"A\";
                inline ic { female: f4; port sig \"SIG\" pin 1; }
                wire red 1 [a, ic.sig];
            }
            view harness \"V\" { include ic.female at (0, 0); }\n",
        )]);
        assert!(problems.is_empty(), "{problems:?}");
        let design = design.unwrap();
        let view = &design.views[0];
        let inc = &view.includes[0];
        assert_eq!(inc.instance.as_str(), "ic");
        assert_eq!(inc.connector.as_ref().map(|c| c.as_str()), Some("ic"));
        assert_eq!(inc.half, Some(Half::Female));
    }

    #[test]
    fn direct_containment_cycle_is_reported() {
        // A single top-level component that instantiates itself.
        let (_design, problems) =
            elaborate_files(&[("main.wb", "component knot { inner: knot; }\n")]);
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::ContainmentCycle { .. })),
            "expected a cycle: {problems:?}"
        );
    }

    #[test]
    fn no_root_when_main_has_no_single_top_level() {
        let (design, problems) =
            elaborate_files(&[("main.wb", "component a { } component b { }\n")]);
        assert!(design.is_none());
        assert!(problems.iter().any(|p| matches!(p, Problem::NoRoot)));
    }
}
