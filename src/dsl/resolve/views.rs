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

use super::{DefId, InlineFacts, Resolver, ViewBinding};

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
            .any(|p| p.connector.map(|c| c.name) == Some(wanted));
        if !exists {
            problems.push(Problem::UnknownConnector {
                name: wanted.to_string(),
                on: format!(" on `{}`", self.defs[tid].name),
                src: self.project.source(file),
                at: conn.span.into(),
            });
        }
    }

    /// A harness include of an inline connector selects a housing half:
    /// `include <inline>.male` or `include <inline>.female`. The half must
    /// be one of those two words and must be declared on the inline; a
    /// `ports { }` block is as wrong here as on any harness include.
    fn check_inline_include(
        &self,
        inc: &ast::Include,
        facts: &InlineFacts<'a>,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let inline = inc.instance.node.as_str();
        let Some(half) = &inc.connector else {
            problems.push(Problem::WrongIncludeForm {
                message: "an inline connector include must select a housing half".to_string(),
                help: format!("write `include {inline}.male` or `include {inline}.female`"),
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
        let wanted = half.node.as_str();
        if wanted != "male" && wanted != "female" {
            problems.push(Problem::UnknownInlineHalf {
                found: wanted.to_string(),
                inline: inline.to_string(),
                src: self.project.source(file),
                at: half.span.into(),
            });
        } else if !facts.declares(wanted) {
            problems.push(Problem::UndeclaredInlineHalf {
                half: wanted.to_string(),
                inline: inline.to_string(),
                src: self.project.source(file),
                at: half.span.into(),
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
                    .any(|p| p.connector.map(|c| c.name) == Some(wanted))
        })
    }

    /// Validate that a view doesn't include the same rendered target twice.
    /// Schematic views render whole instances, while harness views render
    /// instance connectors, so their duplicate keys differ. An inline
    /// connector keys on its name alone — including both halves of one
    /// inline would claim every conductor twice, so it's a duplicate too.
    fn check_duplicate_includes(
        &self,
        view: &ast::View,
        subject: DefId,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let mut seen: HashMap<String, Span> = HashMap::new();
        let view_kind = ViewKind::from(view.kind.node.as_str());
        let is_harness = view_kind.is_harness();
        for inc in &view.includes {
            let name = inc.instance.node.as_str();
            let is_inline = self
                .lookup_inline(subject, &InstanceName::from(name))
                .is_some();
            let target = if is_inline {
                Some(name.to_string())
            } else {
                include_target(inc, is_harness)
            };
            let Some(target) = target else {
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
                if let ViewKind::Other(kind) = &view_kind {
                    problems.push(Problem::UnknownViewKind {
                        kind: kind.clone(),
                        src: self.project.source(FileId(fi)),
                        at: view.kind.span.into(),
                    });
                }
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
                    self.check_duplicate_includes(view, s, FileId(fi), &mut problems);
                    let is_harness = view_kind.is_harness();
                    for inc in &view.includes {
                        if view_kind.is_pinout() {
                            self.check_pinout_include(inc, s, FileId(fi), &mut problems);
                            continue;
                        }
                        let name = inc.instance.node.as_str();
                        match self.lookup_instance(s, &InstanceName::from(name)) {
                            None if is_harness
                                && let Some(facts) =
                                    self.lookup_inline(s, &InstanceName::from(name)) =>
                            {
                                self.check_inline_include(inc, facts, FileId(fi), &mut problems);
                            }
                            None if self.lookup_inline(s, &InstanceName::from(name)).is_some() => {
                                problems.push(Problem::WrongIncludeForm {
                                    message:
                                        "an inline connector renders in harness views, not here"
                                            .to_string(),
                                    help: "include the devices it connects instead, or move \
                                           this include to a harness view"
                                        .to_string(),
                                    src: self.project.source(FileId(fi)),
                                    at: inc.instance.span.into(),
                                });
                            }
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

#[cfg(test)]
mod tests {
    use crate::dsl::diagnostics::Problem;
    use crate::dsl::resolve::testkit::{LEAF, codes, has, inline_project, problems};

    #[test]
    fn unknown_view_kind_is_reported() {
        let p = problems(&[("main.wb", "component m { } view schemtic \"V\" { }\n")]);
        assert!(has(&p, "wirebug::unknown_view_kind"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_view_include_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { include ghost at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_include"), "{:?}", codes(&p));
    }

    #[test]
    fn bare_inline_harness_include_is_wrong_form() {
        let p = inline_project(
            "male: M2; port a \"A\" pin 1;",
            "} view harness \"V\" { include ic at (0, 0); ",
        );
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_inline_half_in_include_is_reported() {
        let p = inline_project(
            "male: M2; port a \"A\" pin 1;",
            "} view harness \"V\" { include ic.plug at (0, 0); ",
        );
        assert!(has(&p, "wirebug::unknown_inline_half"), "{:?}", codes(&p));
    }

    #[test]
    fn undeclared_inline_half_in_include_is_reported() {
        let p = inline_project(
            "male: M2; port a \"A\" pin 1;",
            "} view harness \"V\" { include ic.female at (0, 0); ",
        );
        assert!(
            has(&p, "wirebug::undeclared_inline_half"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn both_halves_in_one_view_is_a_duplicate_include() {
        let p = inline_project(
            "male: M2; female: F2; port a \"A\" pin 1;",
            "} view harness \"V\" { include ic.male at (0, 0); include ic.female at (10, 0); ",
        );
        assert!(
            has(&p, "wirebug::duplicate_view_include"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn schematic_include_of_inline_is_wrong_form() {
        let p = inline_project(
            "male: M2; port a \"A\" pin 1;",
            "} view schematic \"V\" { include ic at (0, 0); ",
        );
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    /// A two-file project whose `main.wb` is a single top-level component
    /// `m` holding one `leaf` instance `l`, plus a view placing `l` with the
    /// given `ports { }` body. `leaf` lives in `leaf.wb`.
    fn view_with_ports(leaf_body: &str, ports_body: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; }}\nview schematic \"V\" {{ include l at (0, 0) ports {{ {ports_body} }} }}\n"
                ),
            ),
            ("leaf.wb", &format!("component leaf {{ {leaf_body} }}\n")),
        ])
    }

    #[test]
    fn unknown_port_side_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "up: a;");
        assert!(has(&p, "wirebug::unknown_port_side"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_port_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "west: nope;");
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn private_port_in_view_is_reported() {
        let p = view_with_ports("port secret \"S\";", "west: secret;");
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_port_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "west: a; east: a;");
        assert!(has(&p, "wirebug::duplicate_view_port"), "{:?}", codes(&p));
    }

    /// Single top-level `m` (a `leaf l` child, a `pub` port `a`, a private
    /// `secret`, wired to the child) with a schematic view carrying the given
    /// `enclosure { }` body. `leaf` lives in `leaf.wb` so `m` is the lone root.
    fn view_with_enclosure(enclosure_body: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; pub port a \"A\"; port secret \"S\"; wire red 1 [a, l.p]; }}\nview schematic \"V\" {{ enclosure {{ {enclosure_body} }} include l at (0, 0) ports {{ west: p; }} }}\n"
                ),
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ])
    }

    #[test]
    fn enclosure_anchor_without_a_side_is_reported() {
        let p = view_with_enclosure("a at (1, 2);");
        assert!(has(&p, "wirebug::enclosure_anchor"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_side_in_the_wrong_slot_is_reported() {
        // north names a horizontal edge, so it belongs in the second slot.
        let p = view_with_enclosure("a at (north, 2);");
        assert!(has(&p, "wirebug::enclosure_anchor"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_unknown_side_is_reported() {
        let p = view_with_enclosure("a at (up, 2);");
        assert!(has(&p, "wirebug::unknown_port_side"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_unknown_subject_port_is_reported() {
        let p = view_with_enclosure("nope at (west, 2);");
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_private_subject_port_is_reported() {
        let p = view_with_enclosure("secret at (west, 2);");
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn well_formed_enclosure_resolves_cleanly() {
        let p = view_with_enclosure("a at (west, 2);");
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn view_body_items_may_appear_in_any_order() {
        // include first, then enclosure, then grid — the reverse of the
        // canonical order — still parses and resolves cleanly.
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; pub port a \"A\"; wire red 1 [a, l.p]; }\nview schematic \"V\" { include l at (0,0) ports { west: p; } enclosure { a at (west, 2); } grid: 10; }\n",
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn duplicate_grid_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { grid: 10; grid: 20; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_item"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_enclosure_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; } view schematic \"V\" { enclosure { a at (west, 0); } enclosure { } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_item"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_text_box_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { text note at (0, 0) \"A\"; text note at (1, 1) \"B\"; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_text"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_schematic_include_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview schematic \"V\" { include l at (0, 0); include l at (2, 0); }\n",
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ]);
        assert!(
            has(&p, "wirebug::duplicate_view_include"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_harness_connector_include_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j1 at (2, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector j1 \"J1\" { pub port p \"P\" pin 1; } }\n",
            ),
        ]);
        assert!(
            has(&p, "wirebug::duplicate_view_include"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn harness_can_include_two_connectors_from_one_instance() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j2 at (2, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector j1 \"J1\" { pub port p \"P\" pin 1; } connector j2 \"J2\" { pub port q \"Q\" pin 1; } }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn text_box_in_harness_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view harness \"H\" { text note at (0, 0) \"A\"; }\n",
        )]);
        assert!(has(&p, "wirebug::unsupported_view_text"), "{:?}", codes(&p));
    }

    /// A two-file project whose `main.wb` holds one `leaf` instance `l` and a
    /// harness view with the given include body. `leaf` (in `leaf.wb`) has a
    /// connector designated `hv`.
    fn harness_view(include: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; }}\nview harness \"H\" {{ {include} }}\n"
                ),
            ),
            (
                "leaf.wb",
                "component leaf { connector hv \"HV 2p\" { pub port hv_pos \"HV+\" pin 1; pub port hv_neg \"HV-\" pin 2; } }\n",
            ),
        ])
    }

    #[test]
    fn harness_include_resolves_cleanly() {
        let p = harness_view("include l.hv at (0, 0);");
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn harness_include_resolves_connector_instance() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "connector_type hv_2p \"HV 2p\" { }\ncomponent leaf { connector hv: hv_2p { pub port hv_pos \"HV+\" pin 1; } }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn unknown_connector_errors() {
        let p = harness_view("include l.nope at (0, 0);");
        assert!(has(&p, "wirebug::unknown_connector"), "{:?}", codes(&p));
    }

    #[test]
    fn harness_include_without_connector_errors() {
        let p = harness_view("include l at (0, 0);");
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn ports_block_on_harness_include_errors() {
        let p = harness_view("include l.hv at (0, 0) ports { west: hv_pos; }");
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn connector_on_schematic_include_errors() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview schematic \"S\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector hv \"HV 2p\" { pub port hv_pos \"HV+\" pin 1; } }\n",
            ),
        ]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn pinout_include_resolves_subject_connector_instance() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn pinout_include_resolves_inline_subject_connector() {
        let p = problems(&[(
            "main.wb",
            "component m {
                connector x1 \"Legacy 1p\" { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn pinout_unknown_connector_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { } view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_connector"), "{:?}", codes(&p));
    }

    #[test]
    fn pinout_include_with_instance_connector_form_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include child.x1 at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn ports_block_on_pinout_include_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0) ports { west: can_h; } }\n",
        )]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    // --- `extend` fragments (split a component across files) ---

    #[test]
    fn fragment_view_includes_an_instance_from_another_fragment() {
        // A schematic in `traction.wb` includes `pack`, which lives in `main`.
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\nextend vehicle { inv: leaf \"Inv\"; }\n\
                 view schematic \"Traction\" {\n\
                   include pack at (2, 2) ports { east: a; }\n\
                   include inv  at (8, 2) ports { west: a; }\n\
                 }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }
}
