//! Elaboration: turn the resolved registry of definitions and instances
//! into a flat-map, hierarchical [`Design`] of concrete instances.
//!
//! Walk from the root definition (main.wb's sole top-level component),
//! stamp one IR instance per placement with a dotted path, materialize
//! its ports, rewrite its wires against the local scope, and recurse into
//! its children. Definitions vanish here; only instances flow to the IR.
//! A type stack guards against containment cycles.

use crate::dsl::ast::{self, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{
    ConnectorRef, Design, Include, Instance, InstanceName, InstancePath, Port, PortName, Side,
    TypeName, View, Visibility, Wire, WireEnd,
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
                            part: c.part.to_string(),
                            index: c.index,
                        }),
                        pins: facts.pins.clone(),
                    },
                )
            })
            .collect();

        let wires = info
            .ast
            .members
            .iter()
            .filter_map(|m| match m {
                Member::Wire(w) => Some(rewrite_wire(w)),
                _ => None,
            })
            .collect();

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
                    includes: v
                        .includes
                        .iter()
                        .map(|inc| Include {
                            instance: InstanceName::from(inc.instance.node.as_str()),
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
                })
            })
            .collect()
    }

    fn resolved_source(&self, def: DefId) -> miette::NamedSource<String> {
        self.resolved.project.source(self.resolved.defs[def].file)
    }
}

fn rewrite_wire(w: &ast::Wire) -> Wire {
    Wire {
        color: w.color.node.as_str().to_string(),
        gauge: w.gauge.node,
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
