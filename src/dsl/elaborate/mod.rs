//! Elaboration: turn the resolved registry of definitions and instances
//! into a flat-map, hierarchical [`Design`] of concrete instances.
//!
//! Walk from the root definition (main.wb's sole top-level component),
//! stamp one IR instance per placement with a dotted path, materialize
//! its ports, rewrite its wires against the local scope, and recurse into
//! its children. Definitions vanish here; only instances flow to the IR.
//! A type stack guards against containment cycles.

use crate::dsl::ast::{self, CablePropertyValue, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, ConnectorRef, Design, EnclosurePort, Include, Instance,
    InstanceName, InstancePath, Port, PortName, Side, TypeName, View, Visibility, Wire, WireEnd,
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

        let info = &self.resolved.defs[def];
        let ports = info
            .ports
            .iter()
            .map(|(name, facts)| {
                (
                    name.clone(),
                    Port {
                        name: name.clone(),
                        label: facts.label.to_string(),
                        visibility: match facts.visibility {
                            ast::Visibility::Public => Visibility::Public,
                            ast::Visibility::Private => Visibility::Private,
                        },
                        connector: facts.connector.as_ref().map(|c| ConnectorRef {
                            name: c.name.map(ConnectorName::from),
                            part: c.part.to_string(),
                            index: c.index,
                        }),
                        pins: facts.pins.clone(),
                    },
                )
            })
            .collect();

        // Loose wires keep `cable: None`; a cable's conductors are tagged with
        // its designator and its metadata recorded separately. Property and
        // arity problems are reported once per def in `validate`, so this pass
        // is best-effort: it takes well-typed values and ignores the rest.
        let mut wires = Vec::new();
        let mut cables: IndexMap<CableName, CableMeta> = IndexMap::new();
        for m in &info.ast.members {
            match m {
                Member::Wire(w) => wires.push(rewrite_wire(w, None)),
                Member::Cable(c) => {
                    let name = CableName::from(c.name.node.as_str());
                    cables.insert(name.clone(), cable_meta(c));
                    for w in &c.wires {
                        wires.push(rewrite_wire(w, Some(name.clone())));
                    }
                }
                _ => {}
            }
        }

        // Resolve child placements (skip instances whose type didn't
        // resolve — that error is already reported).
        let mut children = IndexMap::new();
        let child_jobs: Vec<(InstanceName, DefId, Option<String>)> = info
            .instances
            .iter()
            .filter_map(|(name, inst)| {
                let tid = inst.type_id?;
                let label = inst.ast.label.as_ref().map(|l| l.node.clone());
                Some((name.clone(), tid, label))
            })
            .collect();
        for (name, _, _) in &child_jobs {
            children.insert(name.clone(), path.child(name.clone()));
        }

        self.instances.insert(
            path.clone(),
            Instance {
                path: path.clone(),
                type_name: TypeName::from(info.name),
                label,
                ports,
                children,
                wires,
                cables,
            },
        );

        for (name, tid, label) in child_jobs {
            let child_path = path.child(name);
            self.stamp(tid, &child_path, label, type_stack);
        }

        type_stack.pop();
    }

    fn elaborate_views(&self) -> Vec<View> {
        self.resolved
            .views
            .iter()
            .filter_map(|binding| {
                let subject = binding.subject?;
                let v = binding.ast;
                Some(View {
                    kind: v.kind.node.as_str().to_string(),
                    title: v.title.node.clone(),
                    grid: v.grid.as_ref().map(|g| g.node),
                    subject: TypeName::from(self.resolved.defs[subject].name),
                    // Anchors whose shape resolve already rejected drop here.
                    enclosure: v
                        .enclosure
                        .iter()
                        .filter_map(elaborate_enclosure_port)
                        .collect(),
                    includes: v
                        .includes
                        .iter()
                        .map(|inc| Include {
                            instance: InstanceName::from(inc.instance.node.as_str()),
                            connector: inc
                                .connector
                                .as_ref()
                                .map(|c| ConnectorName::from(c.node.as_str())),
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

fn rewrite_wire(w: &ast::Wire, cable: Option<CableName>) -> Wire {
    Wire {
        color: w.color.node.as_str().to_string(),
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
    fn elaborates_the_seed_design_tree() {
        let main =
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/main.wb"));
        let (project, _) = load(&main);
        let project = project.expect("loads");
        let resolved = resolve(&project);
        let (design, problems) = elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        let design = design.expect("a design");

        assert_eq!(design.root.to_string(), "vehicle");

        // Deep path materialized down through the hierarchy.
        let pack = InstancePath::root(InstanceName::from("vehicle"))
            .child(InstanceName::from("front"))
            .child(InstanceName::from("module_1"))
            .child(InstanceName::from("pack"));
        let pack_inst = design
            .get(&pack)
            .expect("vehicle.front.module_1.pack exists");
        assert_eq!(pack_inst.type_name.as_str(), "cell_pack");
        assert!(pack_inst.ports.contains_key(&PortName::from("hv_pos")));

        // `front` is a front_battery.
        let front =
            InstancePath::root(InstanceName::from("vehicle")).child(InstanceName::from("front"));
        assert_eq!(
            design.get(&front).unwrap().type_name.as_str(),
            "front_battery"
        );

        // The vehicle-level HV bus is a 4-endpoint shared rail; the
        // chassis-ground bus is a 5-endpoint one. Both are multi-endpoint.
        let vehicle = design.get(&design.root).unwrap();
        assert!(
            vehicle.wires.iter().any(|w| w.endpoints.len() == 4),
            "the shared HV bus is a four-endpoint wire"
        );
        assert!(
            vehicle.wires.iter().any(|w| w.endpoints.len() == 5),
            "the chassis-ground bus is a five-endpoint wire"
        );

        // Views came through, bound to their type.
        assert!(design.views.iter().any(|v| v.subject.as_str() == "vehicle"));
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
        let loose = root.wires.iter().find(|w| w.color == "black").unwrap();
        assert!(loose.cable.is_none());
        let conductor = root.wires.iter().find(|w| w.color == "red").unwrap();
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
    fn direct_containment_cycle_is_reported() {
        // A single top-level component that instantiates itself.
        let (_design, problems) =
            elaborate_files(&[("main.wb", "component knot { knot inner; }\n")]);
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
