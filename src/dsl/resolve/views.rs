//! View resolution: bind each view to the component it documents, then
//! validate its includes, `ports { }` placements, and `enclosure { }`
//! anchors. This is the view half of the resolve pass; the definition,
//! scope, and wire-endpoint passes live in the parent [`super`] module and
//! share the same private [`Resolver`].

use std::collections::HashMap;

use crate::dsl::ast::{self, Item};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{ConnectorName, InstanceName, PortName, Side, ViewKind};
use crate::dsl::span::{FileId, Span, Spanned};

use super::{DefId, Resolver, ViewBinding};

impl<'a> Resolver<'a> {
    /// Validate one include's `ports { }` placements against the included
    /// instance's type: side names, duplicate ports, port existence, and
    /// `pub` visibility (mirroring the wire-endpoint rule).
    fn check_view_ports(
        &self,
        inc: &ast::Include,
        type_id: Option<DefId>,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for placement in &inc.ports {
            let side = placement.side.node.as_str();
            if side.parse::<Side>().is_err() {
                problems.push(Problem::UnknownPortSide {
                    found: side.to_string(),
                    src: self.project.source(file),
                    at: placement.side.span.into(),
                });
            }

            let port = placement.port.node.as_str();
            if let Some(&first) = seen.get(port) {
                problems.push(Problem::DuplicateViewPort {
                    port: port.to_string(),
                    src: self.project.source(file),
                    at: placement.port.span.into(),
                    first: first.into(),
                });
            } else {
                seen.insert(port, placement.port.span);
            }

            let Some(tid) = type_id else { continue }; // undefined type already reported
            self.check_port_visibility(tid, port, placement.port.span, file, problems);
        }
    }

    /// Validate that `port` exists and is `pub` on the type `tid`, pushing the
    /// matching problem otherwise. Shared by include and enclosure checks.
    fn check_port_visibility(
        &self,
        tid: DefId,
        port: &str,
        span: Span,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        match self.lookup_port(tid, &PortName::from(port)) {
            None => problems.push(Problem::UnknownPort {
                port: port.to_string(),
                on: format!(" on `{}`", self.defs[tid].name),
                src: self.project.source(file),
                at: span.into(),
            }),
            Some(facts) if facts.visibility != ast::Visibility::Public => {
                problems.push(Problem::PrivatePort {
                    port: port.to_string(),
                    ty: self.defs[tid].name.to_string(),
                    src: self.project.source(file),
                    at: span.into(),
                });
            }
            Some(_) => {}
        }
    }

    /// Validate the view's `enclosure { }` block against the subject type:
    /// each `<port> at (x, y)` must name exactly one side (west/east in the x
    /// slot, north/south in the y slot) and one coordinate, the port must
    /// exist and be `pub` on the subject, and no port is placed twice.
    fn check_enclosure(
        &self,
        view: &ast::View,
        subject: DefId,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for ep in &view.enclosure {
            self.check_enclosure_anchor(ep, file, problems);

            let port = ep.port.node.as_str();
            if let Some(&first) = seen.get(port) {
                problems.push(Problem::DuplicateViewPort {
                    port: port.to_string(),
                    src: self.project.source(file),
                    at: ep.port.span.into(),
                    first: first.into(),
                });
            } else {
                seen.insert(port, ep.port.span);
            }

            self.check_port_visibility(subject, port, ep.port.span, file, problems);
        }
    }

    /// Check an enclosure port's `(x, y)` anchor: exactly one slot is a side
    /// and one a coordinate, and the side sits in the slot for its axis.
    fn check_enclosure_anchor(
        &self,
        ep: &ast::EnclosurePort,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        use ast::Anchor::{Coord, Edge};
        match (&ep.x, &ep.y) {
            (Edge(s), Coord(_)) => self.check_edge_slot(s, true, file, problems),
            (Coord(_), Edge(s)) => self.check_edge_slot(s, false, file, problems),
            (Coord(_), Coord(_)) => problems.push(Problem::EnclosureAnchor {
                message: "an enclosure port anchor must name one side; neither slot is a side"
                    .to_string(),
                src: self.project.source(file),
                at: ep.span.into(),
            }),
            (Edge(_), Edge(_)) => problems.push(Problem::EnclosureAnchor {
                message:
                    "an enclosure port anchor names one side and one coordinate, not two sides"
                        .to_string(),
                src: self.project.source(file),
                at: ep.span.into(),
            }),
        }
    }

    /// Validate a side keyword found in an anchor slot. `x_slot` is true for
    /// the first slot (which takes west/east), false for the second
    /// (north/south).
    fn check_edge_slot(
        &self,
        side: &Spanned<ast::Ident>,
        x_slot: bool,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let s = side.node.as_str();
        match s.parse::<Side>() {
            Err(()) => problems.push(Problem::UnknownPortSide {
                found: s.to_string(),
                src: self.project.source(file),
                at: side.span.into(),
            }),
            Ok(parsed) => {
                let fits = if x_slot {
                    matches!(parsed, Side::West | Side::East)
                } else {
                    matches!(parsed, Side::North | Side::South)
                };
                if !fits {
                    let message = if x_slot {
                        format!(
                            "`{s}` is a horizontal edge; put it in the second slot: `(<x>, {s})`"
                        )
                    } else {
                        format!("`{s}` is a vertical edge; put it in the first slot: `({s}, <y>)`")
                    };
                    problems.push(Problem::EnclosureAnchor {
                        message,
                        src: self.project.source(file),
                        at: side.span.into(),
                    });
                }
            }
        }
    }

    /// A schematic include names a bare instance and authors `ports`
    /// placements; naming a connector here is the wrong form.
    fn check_schematic_include(
        &self,
        inc: &ast::Include,
        type_id: Option<DefId>,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        if let Some(conn) = &inc.connector {
            problems.push(Problem::WrongIncludeForm {
                message: "a schematic include names a bare instance, not a connector".to_string(),
                help: "drop the `.connector`; schematic boxes show ports via a `ports { }` block"
                    .to_string(),
                src: self.project.source(file),
                at: conn.span.into(),
            });
        }
        self.check_view_ports(inc, type_id, file, problems);
    }

    /// A harness include names `instance.connector` and draws that whole
    /// connector. It must carry a connector, must not carry a `ports { }`
    /// block, and the connector must exist on the instance's type.
    fn check_harness_include(
        &self,
        inc: &ast::Include,
        type_id: Option<DefId>,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let Some(conn) = &inc.connector else {
            problems.push(Problem::WrongIncludeForm {
                message: "a harness include must name a connector".to_string(),
                help: "write `include <instance>.<connector> at (x, y);`".to_string(),
                src: self.project.source(file),
                at: inc.instance.span.into(),
            });
            return;
        };
        if let Some(first) = inc.ports.first() {
            problems.push(Problem::WrongIncludeForm {
                message: "a harness include draws a whole connector, not selected ports"
                    .to_string(),
                help: "remove the `ports { }` block from this harness include".to_string(),
                src: self.project.source(file),
                at: first.span.into(),
            });
        }
        let Some(tid) = type_id else { return }; // undefined type already reported
        let wanted = conn.node.as_str();
        let exists = self.defs[tid]
            .ports
            .values()
            .any(|p| p.connector.and_then(|c| c.name) == Some(wanted));
        if !exists {
            problems.push(Problem::UnknownConnector {
                name: wanted.to_string(),
                on: format!(" on `{}`", self.defs[tid].name),
                src: self.project.source(file),
                at: conn.span.into(),
            });
        }
    }

    /// A pinout include names a connector on the view subject itself:
    /// `include <connector> at (x, y);`. It carries neither an instance
    /// connector segment nor a schematic `ports { }` block.
    fn check_pinout_include(
        &self,
        inc: &ast::Include,
        subject: DefId,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        if let Some(connector) = &inc.connector {
            problems.push(Problem::WrongIncludeForm {
                message: "a pinout include names a subject connector, not an instance connector"
                    .to_string(),
                help: "write `include <connector> at (x, y);`".to_string(),
                src: self.project.source(file),
                at: connector.span.into(),
            });
        }
        if let Some(first) = inc.ports.first() {
            problems.push(Problem::WrongIncludeForm {
                message: "a pinout include draws a whole connector, not selected ports".to_string(),
                help: "remove the `ports { }` block from this pinout include".to_string(),
                src: self.project.source(file),
                at: first.span.into(),
            });
        }

        let wanted = inc.instance.node.as_str();
        if !self.connector_exists(subject, wanted) {
            problems.push(Problem::UnknownConnector {
                name: wanted.to_string(),
                on: format!(" on `{}`", self.defs[subject].name),
                src: self.project.source(file),
                at: inc.instance.span.into(),
            });
        }
    }

    fn connector_exists(&self, tid: DefId, wanted: &str) -> bool {
        // A connector may be declared in any fragment of a merged component.
        self.groups.fragments(tid).into_iter().any(|f| {
            self.defs[f]
                .connectors
                .contains_key(&ConnectorName::from(wanted))
                || self.defs[f]
                    .ports
                    .values()
                    .any(|p| p.connector.and_then(|c| c.name) == Some(wanted))
        })
    }

    /// Validate that a view doesn't include the same rendered target twice.
    /// Schematic views render whole instances, while harness views render
    /// instance connectors, so their duplicate keys differ.
    fn check_duplicate_includes(
        &self,
        view: &ast::View,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let mut seen: HashMap<String, Span> = HashMap::new();
        let view_kind = ViewKind::from(view.kind.node.as_str());
        let is_harness = view_kind.is_harness();
        for inc in &view.includes {
            let Some(target) = include_target(inc, is_harness) else {
                continue;
            };
            if let Some(&first) = seen.get(&target) {
                problems.push(Problem::DuplicateViewInclude {
                    target,
                    src: self.project.source(file),
                    at: inc.instance.span.into(),
                    first: first.into(),
                });
            } else {
                seen.insert(target, inc.instance.span);
            }
        }
    }

    pub(super) fn resolve_views(&mut self, roots_by_file: &[Vec<DefId>]) -> Vec<ViewBinding<'a>> {
        let mut bindings = Vec::new();
        let mut problems = Vec::new();
        for (fi, file) in self.project.files.iter().enumerate() {
            let roots = &roots_by_file[fi];
            for item in &file.ast.items {
                let Item::View(view) = item else { continue };
                let view_kind = ViewKind::from(view.kind.node.as_str());
                for dup in &view.duplicate_items {
                    problems.push(Problem::DuplicateViewItem {
                        kind: dup.node.to_string(),
                        src: self.project.source(FileId(fi)),
                        at: dup.span.into(),
                    });
                }
                let mut text_names: HashMap<&str, Span> = HashMap::new();
                for text in &view.texts {
                    let name = text.name.node.as_str();
                    if let Some(first) = text_names.insert(name, text.name.span) {
                        problems.push(Problem::DuplicateViewText {
                            name: name.to_string(),
                            src: self.project.source(FileId(fi)),
                            at: text.name.span.into(),
                            first: first.into(),
                        });
                    }
                    if !view_kind.is_schematic() {
                        problems.push(Problem::UnsupportedViewText {
                            kind: view_kind.to_string(),
                            src: self.project.source(FileId(fi)),
                            at: text.span.into(),
                        });
                    }
                }
                // A view in a fragment file binds to that fragment's top-level
                // definition; canonicalize so it documents — and sees the whole
                // merged namespace of — the one component it extends.
                let subject = if roots.len() == 1 {
                    Some(self.groups.canonical(roots[0]))
                } else {
                    problems.push(Problem::ViewSubject {
                        src: self.project.source(FileId(fi)),
                        at: view.span.into(),
                    });
                    None
                };
                if let Some(s) = subject {
                    self.check_enclosure(view, s, FileId(fi), &mut problems);
                    self.check_duplicate_includes(view, FileId(fi), &mut problems);
                    let is_harness = view_kind.is_harness();
                    for inc in &view.includes {
                        if view_kind.is_pinout() {
                            self.check_pinout_include(inc, s, FileId(fi), &mut problems);
                            continue;
                        }
                        let name = inc.instance.node.as_str();
                        match self.lookup_instance(s, &InstanceName::from(name)) {
                            None => problems.push(Problem::UnknownInclude {
                                name: name.to_string(),
                                src: self.project.source(FileId(fi)),
                                at: inc.instance.span.into(),
                            }),
                            Some(facts) if is_harness => self.check_harness_include(
                                inc,
                                facts.type_id,
                                FileId(fi),
                                &mut problems,
                            ),
                            Some(facts) => self.check_schematic_include(
                                inc,
                                facts.type_id,
                                FileId(fi),
                                &mut problems,
                            ),
                        }
                    }
                }
                bindings.push(ViewBinding { ast: view, subject });
            }
        }
        self.problems.extend(problems);
        bindings
    }
}

fn include_target(inc: &ast::Include, is_harness: bool) -> Option<String> {
    let instance = inc.instance.node.as_str();
    if is_harness {
        let connector = inc.connector.as_ref()?.node.as_str();
        Some(format!("{instance}.{connector}"))
    } else {
        Some(instance.to_string())
    }
}
