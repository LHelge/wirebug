//! chumsky parser: a significant `(Token, Span)` stream → an [`ast::File`].
//!
//! Built bottom-up from small combinators. The only recursive production
//! is `definition` (components nest), handled with [`recursive`]. Errors
//! are chumsky `Rich` values carrying a span and an expected-set; we copy
//! them into owned [`ParseError`]s so no parser lifetime escapes.

use chumsky::input::{Input, Stream, ValueInput};
use chumsky::prelude::{IterParser, Parser, Rich, choice, end, extra, just, recursive, select};

use crate::dsl::ast::*;
use crate::dsl::lex::Token;
use crate::dsl::span::{FileId, Span, Spanned};

/// Use our file-tagged [`Span`] as chumsky's span type, so `e.span()`
/// yields spans that already know their file.
impl chumsky::span::Span for Span {
    type Context = FileId;
    type Offset = usize;

    fn new(context: FileId, range: std::ops::Range<usize>) -> Self {
        Span {
            file: context,
            start: range.start,
            end: range.end,
        }
    }

    fn context(&self) -> FileId {
        self.file
    }

    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }
}

/// An owned parse error (span + rendered message), independent of any
/// parser lifetime. Becomes a miette diagnostic in the diagnostics layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

/// Result of parsing one file: the AST (absent only on unrecoverable
/// failure) and any errors encountered.
#[derive(Debug)]
pub struct Parsed {
    pub file: Option<File>,
    pub errors: Vec<ParseError>,
}

impl Parsed {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// One member of a view body, parsed in any order and folded into a [`View`].
enum ViewItem {
    Grid(Spanned<f64>),
    Enclosure(Spanned<Vec<EnclosurePort>>),
    Include(Include),
    Text(TextBox),
}

/// One member of a cable body, parsed in any order and folded into a
/// [`Cable`]: a property (`type`/`length`) or a conductor wire.
enum CableItem {
    Property(CableProperty),
    Wire(Wire),
}

/// One top-level file entry, parsed in source order and folded into
/// [`File::uses`] plus [`File::items`].
enum FileEntry {
    Use(Use),
    Item(Item),
}

/// Parse the significant token stream of one file into an [`ast::File`].
pub fn parse_file(tokens: Vec<(Token, Span)>, file: FileId, src_len: usize) -> Parsed {
    let eoi = Span {
        file,
        start: src_len,
        end: src_len,
    };
    let stream = Stream::from_iter(tokens).map(eoi, |(t, s)| (t, s));
    let (file, errors) = parser().parse(stream).into_output_errors();
    Parsed {
        file,
        errors: errors
            .into_iter()
            .map(|e| ParseError {
                span: *e.span(),
                message: e.reason().to_string(),
            })
            .collect(),
    }
}

#[allow(clippy::type_complexity)]
fn parser<'tok, I>() -> impl Parser<'tok, I, File, extra::Err<Rich<'tok, Token, Span>>>
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    // --- Leaves ---

    let ident = select! { Token::Ident(name) => name }
        .map_with(|name, e| Spanned::new(Ident(name), e.span()));

    let string = select! { Token::Str(s) => s }.map_with(|s, e| Spanned::new(s, e.span()));

    let number = select! { Token::Number(n) => n }.try_map(|n, span: Span| {
        n.parse::<f64>()
            .map(|v| Spanned::new(v, span))
            .map_err(|_| Rich::custom(span, format!("invalid number `{n}`")))
    });

    let pin = select! { Token::Number(n) => n }.try_map(|n, span: Span| {
        n.parse::<u32>()
            .map(|v| Spanned::new(v, span))
            .map_err(|_| Rich::custom(span, format!("pin must be a whole number, got `{n}`")))
    });

    // --- Ports & connectors ---

    let single_pin = just(Token::Pin).ignore_then(pin).map(|p| vec![p]);
    let multi_pins = just(Token::Pins).ignore_then(
        pin.separated_by(just(Token::Comma))
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen)),
    );
    let pins = choice((single_pin, multi_pins))
        .or_not()
        .map(|o| o.unwrap_or_default());

    let port = just(Token::Pub)
        .or_not()
        .then_ignore(just(Token::Port))
        .then(ident)
        .then(string)
        .then(pins)
        .then_ignore(just(Token::Semicolon))
        .map_with(|(((vis, name), label), pins), e| Port {
            visibility: if vis.is_some() {
                Visibility::Public
            } else {
                Visibility::Private
            },
            name,
            label,
            pins,
            span: e.span(),
        });

    let connector = just(Token::Connector)
        .ignore_then(ident.or_not())
        .then(string)
        .then(
            port.clone()
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((name, part), ports), e| Connector {
            name,
            part,
            ports,
            span: e.span(),
        });

    // --- Instances ---

    let instance = ident
        .then(ident)
        .then(string.or_not())
        .then_ignore(just(Token::Semicolon))
        .map_with(|((type_name, name), label), e| Instance {
            type_name,
            name,
            label,
            span: e.span(),
        });

    // --- Wires ---

    let endpoint = ident
        .then(just(Token::Dot).ignore_then(ident).or_not())
        .map_with(|(first, second), e| match second {
            Some(port) => Endpoint {
                instance: Some(first),
                port,
                span: e.span(),
            },
            None => Endpoint {
                instance: None,
                port: first,
                span: e.span(),
            },
        });

    let wire = just(Token::Wire)
        .ignore_then(ident)
        .then(number)
        .then(string.or_not())
        .then(
            endpoint
                .separated_by(just(Token::Comma))
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBracket), just(Token::RBracket)),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|(((color, gauge), label), endpoints), e| Wire {
            color,
            gauge,
            label,
            endpoints,
            span: e.span(),
        });

    // --- Cables ---

    let cable_property = ident
        .then_ignore(just(Token::Colon))
        .then(choice((
            string.map(CablePropertyValue::Str),
            number.map(CablePropertyValue::Number),
        )))
        .then_ignore(just(Token::Semicolon))
        .map_with(|(key, value), e| CableProperty {
            key,
            value,
            span: e.span(),
        });

    // Properties and conductor wires may interleave in any order; the fold
    // partitions them, each keeping its source order.
    let cable_item = choice((
        cable_property.map(CableItem::Property),
        wire.clone().map(CableItem::Wire),
    ));

    let cable = just(Token::Cable)
        .ignore_then(ident)
        .then(string.or_not())
        .then(
            cable_item
                .repeated()
                .collect::<Vec<CableItem>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((name, label), items), e| {
            let mut properties = Vec::new();
            let mut wires = Vec::new();
            for item in items {
                match item {
                    CableItem::Property(p) => properties.push(p),
                    CableItem::Wire(w) => wires.push(w),
                }
            }
            Cable {
                name,
                label,
                properties,
                wires,
                span: e.span(),
            }
        });

    // --- Views ---

    // A `<side>: <port>, <port>;` line, flattened to one placement per port
    // (all sharing the line's side and span).
    let port_line = ident
        .then_ignore(just(Token::Colon))
        .then(
            ident
                .separated_by(just(Token::Comma))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|(side, ports), e| {
            let span = e.span();
            ports
                .into_iter()
                .map(move |port| PortPlacement {
                    side: side.clone(),
                    port,
                    span,
                })
                .collect::<Vec<_>>()
        });

    let ports_block = just(Token::Ports)
        .ignore_then(
            port_line
                .repeated()
                .collect::<Vec<Vec<PortPlacement>>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map(|lines| lines.into_iter().flatten().collect::<Vec<PortPlacement>>());

    // One anchor slot: a grid coordinate or a side keyword. Resolve checks
    // that exactly one of an enclosure port's two slots is a side.
    let anchor = choice((number.map(Anchor::Coord), ident.map(Anchor::Edge)));

    // `<port> at (<x>, <y>);` — a subject port placed on the enclosure edge.
    let enclosure_port = ident
        .then_ignore(just(Token::At))
        .then(
            anchor
                .then_ignore(just(Token::Comma))
                .then(anchor)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|(port, (x, y)), e| EnclosurePort {
            port,
            x,
            y,
            span: e.span(),
        });

    let enclosure_block = just(Token::Enclosure)
        .ignore_then(
            enclosure_port
                .repeated()
                .collect::<Vec<EnclosurePort>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|ports, e| Spanned::new(ports, e.span()));

    let include = just(Token::Include)
        .ignore_then(ident)
        .then(just(Token::Dot).ignore_then(ident).or_not())
        .then_ignore(just(Token::At))
        .then(
            number
                .then_ignore(just(Token::Comma))
                .then(number)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(ports_block.or_not())
        .then_ignore(just(Token::Semicolon))
        .map_with(|(((instance, connector), (x, y)), ports), e| Include {
            instance,
            connector,
            x,
            y,
            ports: ports.unwrap_or_default(),
            span: e.span(),
        });

    let text_box = just(Token::Text)
        .ignore_then(ident)
        .then_ignore(just(Token::At))
        .then(
            number
                .then_ignore(just(Token::Comma))
                .then(number)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(string)
        .then_ignore(just(Token::Semicolon))
        .map_with(|((name, (x, y)), label), e| TextBox {
            name,
            x,
            y,
            label,
            span: e.span(),
        });

    let grid = just(Token::Grid)
        .ignore_then(number)
        .then_ignore(just(Token::Semicolon));

    // The view body is a free-order list of these; the fold below keeps the
    // first `grid`/`enclosure` and records any extras for a duplicate report.
    let view_item = choice((
        grid.map(ViewItem::Grid),
        enclosure_block.map(ViewItem::Enclosure),
        include.map(ViewItem::Include),
        text_box.map(ViewItem::Text),
    ));

    let view = just(Token::View)
        .ignore_then(ident)
        .then(string)
        .then(
            view_item
                .repeated()
                .collect::<Vec<ViewItem>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((kind, title), items), e| {
            let mut grid = None;
            let mut enclosure = None;
            let mut includes = Vec::new();
            let mut texts = Vec::new();
            let mut duplicate_items = Vec::new();
            for item in items {
                match item {
                    ViewItem::Grid(g) if grid.is_some() => {
                        duplicate_items.push(Spanned::new("grid", g.span));
                    }
                    ViewItem::Grid(g) => grid = Some(g),
                    ViewItem::Enclosure(block) if enclosure.is_some() => {
                        duplicate_items.push(Spanned::new("enclosure", block.span));
                    }
                    ViewItem::Enclosure(block) => enclosure = Some(block.node),
                    ViewItem::Include(inc) => includes.push(inc),
                    ViewItem::Text(text) => texts.push(text),
                }
            }
            View {
                kind,
                title,
                grid,
                enclosure: enclosure.unwrap_or_default(),
                includes,
                texts,
                duplicate_items,
                span: e.span(),
            }
        });

    // --- Definitions (recursive: components nest) ---

    let definition = recursive(|definition| {
        let member = choice((
            port.map(Member::Port),
            connector.map(Member::Connector),
            wire.map(Member::Wire),
            cable.map(Member::Cable),
            definition.map(Member::Definition),
            instance.map(Member::Instance),
        ));
        just(Token::Component)
            .ignore_then(ident)
            .then(
                member
                    .repeated()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map_with(|(name, members), e| Definition {
                name,
                members,
                span: e.span(),
            })
    });

    // --- File ---

    let item = choice((definition.map(Item::Definition), view.map(Item::View)));

    let use_decl = just(Token::Use)
        .ignore_then(ident)
        .then_ignore(just(Token::From))
        .then(string)
        .map_with(|(name, path), e| Use {
            name,
            path,
            span: e.span(),
        });

    let file_entry = choice((use_decl.map(FileEntry::Use), item.map(FileEntry::Item)));

    file_entry
        .repeated()
        .collect::<Vec<FileEntry>>()
        .then_ignore(end())
        .map_with(|entries, e| {
            let mut uses = Vec::new();
            let mut items = Vec::new();
            for entry in entries {
                match entry {
                    FileEntry::Use(use_decl) => uses.push(use_decl),
                    FileEntry::Item(item) => items.push(item),
                }
            }
            let span: Span = e.span();
            File {
                id: span.file,
                uses,
                items,
                span,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::lex::{lex, significant};

    fn parse_str(src: &str) -> Parsed {
        let lexemes = lex(src, FileId(0)).expect("lexes");
        parse_file(significant(&lexemes), FileId(0), src.len())
    }

    fn parse_ok(src: &str) -> File {
        let parsed = parse_str(src);
        assert!(
            parsed.errors.is_empty(),
            "unexpected parse errors: {:?}",
            parsed.errors
        );
        parsed.file.expect("a file")
    }

    /// Members of the sole top-level definition.
    fn members(file: &File) -> &[Member] {
        match &file.items[0] {
            Item::Definition(d) => &d.members,
            other => panic!("expected a definition, got {other:?}"),
        }
    }

    #[test]
    fn pub_port_with_pin() {
        let file = parse_ok(r#"component c { pub port hv_pos "HV+" pin 1; }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(p.visibility, Visibility::Public);
        assert_eq!(p.name.node.as_str(), "hv_pos");
        assert_eq!(p.label.node, "HV+");
        assert_eq!(p.pins.iter().map(|s| s.node).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn bare_private_port_has_no_pins() {
        let file = parse_ok(r#"component c { port coil_n "COIL-"; }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(p.visibility, Visibility::Private);
        assert!(p.pins.is_empty());
    }

    #[test]
    fn port_with_ganged_pins() {
        let file = parse_ok(r#"component c { pub port gnd "GND" pins (2, 3, 4); }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(
            p.pins.iter().map(|s| s.node).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn connector_groups_ports() {
        let file = parse_ok(
            r#"component c { connector "CAN 4p" { pub port can_h "CAN H" pin 1; pub port can_l "CAN L" pin 2; } }"#,
        );
        let Member::Connector(conn) = &members(&file)[0] else {
            panic!("expected a connector");
        };
        assert_eq!(conn.part.node, "CAN 4p");
        assert_eq!(conn.ports.len(), 2);
        assert!(conn.name.is_none(), "connector designator is optional");
    }

    #[test]
    fn connector_carries_designator() {
        let file =
            parse_ok(r#"component c { connector hv "HV DC 2p" { pub port p "DC+" pin 1; } }"#);
        let Member::Connector(conn) = &members(&file)[0] else {
            panic!("expected a connector");
        };
        assert_eq!(conn.name.as_ref().map(|n| n.node.as_str()), Some("hv"));
        assert_eq!(conn.part.node, "HV DC 2p");
    }

    #[test]
    fn wire_carries_label() {
        let file = parse_ok(r#"component c { wire orange 50 "HV+" [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.label.as_ref().map(|l| l.node.as_str()), Some("HV+"));
        assert_eq!(w.endpoints.len(), 2);
        // Unlabelled wires still parse.
        let bare = parse_ok(r#"component c { wire orange 50 [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&bare)[0] else {
            panic!("expected wire");
        };
        assert!(w.label.is_none());
    }

    #[test]
    fn cable_groups_wires_with_properties() {
        let file = parse_ok(
            r#"component c {
                cable can_bus "Vehicle CAN" {
                    type:   "Twisted pair";
                    length: 2.5;
                    wire yellow 0.5 "CAN H" [vcu.can_h, inv.can_h];
                    wire green  0.5 "CAN L" [vcu.can_l, inv.can_l];
                }
            }"#,
        );
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert_eq!(cable.name.node.as_str(), "can_bus");
        assert_eq!(
            cable.label.as_ref().map(|l| l.node.as_str()),
            Some("Vehicle CAN")
        );
        assert_eq!(cable.properties.len(), 2);
        assert_eq!(cable.properties[0].key.node.as_str(), "type");
        assert!(matches!(
            cable.properties[1].value,
            CablePropertyValue::Number(_)
        ));
        assert_eq!(cable.wires.len(), 2);
    }

    #[test]
    fn cable_properties_and_wires_may_interleave() {
        // A wire before a property, and a property between wires — the fold
        // partitions them, each side keeping source order.
        let file = parse_ok(
            r#"component c {
                cable can_bus {
                    wire yellow 0.5 [vcu.can_h, inv.can_h];
                    type:   "Twisted pair";
                    wire green  0.5 [vcu.can_l, inv.can_l];
                    length: 2.5;
                }
            }"#,
        );
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert_eq!(cable.properties.len(), 2);
        assert_eq!(cable.properties[0].key.node.as_str(), "type");
        assert_eq!(cable.wires.len(), 2);
        assert_eq!(cable.wires[0].color.node.as_str(), "yellow");
        assert_eq!(cable.wires[1].color.node.as_str(), "green");
    }

    #[test]
    fn cable_without_label_or_properties_parses() {
        let file = parse_ok(r#"component c { cable cab { wire red 1.0 [a.p, b.p]; } }"#);
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert!(cable.label.is_none());
        assert!(cable.properties.is_empty());
        assert_eq!(cable.wires.len(), 1);
    }

    #[test]
    fn harness_include_references_connector() {
        let file = parse_ok(r#"view harness "H" { include charger.hv at (2, 4); }"#);
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.kind.node.as_str(), "harness");
        let inc = &v.includes[0];
        assert_eq!(inc.instance.node.as_str(), "charger");
        assert_eq!(inc.connector.as_ref().map(|c| c.node.as_str()), Some("hv"));
        assert!(inc.ports.is_empty());
    }

    #[test]
    fn instance_with_and_without_label() {
        let file =
            parse_ok(r#"component c { cell_pack pack; front_battery front "Front Battery"; }"#);
        let Member::Instance(a) = &members(&file)[0] else {
            panic!("expected instance");
        };
        assert_eq!(a.type_name.node.as_str(), "cell_pack");
        assert_eq!(a.name.node.as_str(), "pack");
        assert!(a.label.is_none());
        let Member::Instance(b) = &members(&file)[1] else {
            panic!("expected instance");
        };
        assert_eq!(
            b.label.as_ref().map(|l| l.node.as_str()),
            Some("Front Battery")
        );
    }

    #[test]
    fn wire_endpoints_bare_and_qualified() {
        let file = parse_ok(r#"component c { wire orange 50 [hv_pos, pack.hv_pos, inv.dc_pos]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.color.node.as_str(), "orange");
        assert_eq!(w.gauge.node, 50.0);
        assert_eq!(w.endpoints.len(), 3);
        // bare self-port
        assert!(w.endpoints[0].instance.is_none());
        assert_eq!(w.endpoints[0].port.node.as_str(), "hv_pos");
        // instance-qualified
        assert_eq!(
            w.endpoints[1].instance.as_ref().unwrap().node.as_str(),
            "pack"
        );
        assert_eq!(w.endpoints[1].port.node.as_str(), "hv_pos");
    }

    #[test]
    fn fractional_gauge_parses() {
        let file = parse_ok(r#"component c { wire black 0.25 [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.gauge.node, 0.25);
    }

    #[test]
    fn nested_definition() {
        let file = parse_ok(r#"component outer { component inner { pub port a "A"; } inner i; }"#);
        let ms = members(&file);
        assert!(matches!(ms[0], Member::Definition(_)));
        assert!(matches!(ms[1], Member::Instance(_)));
    }

    #[test]
    fn view_with_and_without_grid() {
        let file = parse_ok(
            r#"view schematic "Overview" { grid 20; include a at (3, 4); include b at (5, 6); }"#,
        );
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.kind.node.as_str(), "schematic");
        assert_eq!(v.title.node, "Overview");
        assert_eq!(v.grid.as_ref().map(|g| g.node), Some(20.0));
        assert_eq!(v.includes.len(), 2);
        assert_eq!(v.includes[0].instance.node.as_str(), "a");
        assert_eq!((v.includes[0].x.node, v.includes[0].y.node), (3.0, 4.0));

        let no_grid = parse_ok(r#"view schematic "X" { include a at (0, 0); }"#);
        let Item::View(v) = &no_grid.items[0] else {
            panic!("expected view");
        };
        assert!(v.grid.is_none());
        assert!(v.includes[0].ports.is_empty(), "bare include has no ports");
    }

    #[test]
    fn include_ports_block_flattens_in_order() {
        let file = parse_ok(
            r#"view schematic "V" { include a at (1, 2) ports { west: p, q; east: r; }; }"#,
        );
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        let placements = &v.includes[0].ports;
        let got: Vec<(&str, &str)> = placements
            .iter()
            .map(|pl| (pl.side.node.as_str(), pl.port.node.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![("west", "p"), ("west", "q"), ("east", "r")],
            "placements keep their declaration order, one per port"
        );
    }

    #[test]
    fn view_text_box_parses() {
        let file =
            parse_ok(r#"view schematic "V" { text note_1 at (3, 4) "This is my textbox!"; }"#);
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.texts.len(), 1);
        assert_eq!(v.texts[0].name.node.as_str(), "note_1");
        assert_eq!((v.texts[0].x.node, v.texts[0].y.node), (3.0, 4.0));
        assert_eq!(v.texts[0].label.node, "This is my textbox!");
    }

    #[test]
    fn use_declaration() {
        let file = parse_ok(
            "use cell_module from \"components/cell_module.wb\"\ncomponent c { pub port a \"A\"; }",
        );
        assert_eq!(file.uses.len(), 1);
        assert_eq!(file.uses[0].name.node.as_str(), "cell_module");
        assert_eq!(file.uses[0].path.node, "components/cell_module.wb");
    }

    #[test]
    fn use_declarations_may_interleave_with_items() {
        let file = parse_ok(
            r#"
component a { }
use leaf from "leaf.wb"
view schematic "A" { }
use relay from "relay.wb"
component b { }
"#,
        );

        let uses: Vec<&str> = file.uses.iter().map(|u| u.name.node.as_str()).collect();
        assert_eq!(uses, vec!["leaf", "relay"]);
        assert_eq!(file.items.len(), 3);
        assert!(matches!(file.items[0], Item::Definition(_)));
        assert!(matches!(file.items[1], Item::View(_)));
        assert!(matches!(file.items[2], Item::Definition(_)));
    }

    #[test]
    fn missing_semicolon_is_a_parse_error() {
        let parsed = parse_str(r#"component c { pub port a "A" }"#);
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn parses_every_example_file() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
        let mut count = 0;
        for path in super::tests::walk_wb(std::path::Path::new(root)) {
            let src = std::fs::read_to_string(&path).expect("read example");
            let parsed = parse_str(&src);
            assert!(
                parsed.errors.is_empty() && parsed.file.is_some(),
                "parse {}: {:?}",
                path.display(),
                parsed.errors
            );
            count += 1;
        }
        assert!(count >= 13, "expected the full seed corpus, found {count}");
    }

    fn walk_wb(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                out.extend(walk_wb(&path));
            } else if path.extension().is_some_and(|e| e == "wb") {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn contactor_ast_snapshot() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/components/contactor.wb"
        );
        let src = std::fs::read_to_string(path).expect("read contactor.wb");
        let file = parse_ok(&src);
        insta::assert_debug_snapshot!(file);
    }
}
