//! A port's pin annotation: the pin number(s) drawn outside the box beside
//! the port. On a component with several connectors a number alone is
//! ambiguous (an HV and an interlock connector each have a pin 1), so it is
//! qualified with the port's connector designator: `hv:1`.

use super::draw::LabelPlacement;
use super::{PIN_ACROSS, PIN_CHAR_WIDTH, PIN_INSET, PIN_NEIGHBOUR_GAP, PIN_TITLE_GAP};
use crate::dsl::ir::{Instance, Pin, Port};
use crate::render::geometry::{Bounds, Point, Side};

/// The pin font's size, and how far its glyphs reach above and below an
/// alphabetic baseline: a sans-serif face's ascender and descender
/// (Arial's 0.905 and 0.212 em, rounded up) rather than the ink of the
/// digits, so an extent never undershoots the text it measures.
const FONT_SIZE: f64 = 10.0;
const ASCENT: f64 = 9.1;
const DESCENT: f64 = 2.2;

/// Which way a pin annotation reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PinOrientation {
    /// Level text. Beside a west or east port it runs outward along the
    /// wire stub, above it; beside a north or south port it runs across
    /// the stub, toward the next port on the side.
    Horizontal,
    /// Turned to run along a north or south port's stub, so its length
    /// runs away from the box instead of toward the next port. It reads
    /// toward the port (downward on a north side, upward on a south one),
    /// so the pin number, at its end, sits next to the port.
    Vertical,
}

/// The text beside one port and the way it reads.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PinAnnotation {
    pub(super) text: String,
    pub(super) orientation: PinOrientation,
    /// How much farther out along a north stub than usual the text sits,
    /// to clear the component title ([`Self::clear_title`]); 0 elsewhere.
    lift: f64,
}

impl PinAnnotation {
    /// `text` read the way `orientation` says, in its usual place.
    pub(super) fn new(text: String, orientation: PinOrientation) -> Self {
        Self {
            text,
            orientation,
            lift: 0.0,
        }
    }

    /// Move the text of a north port at `pos` out along its stub, past the
    /// component's `title`, when it would meet it there: its end nearest
    /// the port (the pin number of a turned text) then sits at least
    /// `PIN_TITLE_GAP` above the title (the move rounded up to a
    /// whole unit, so the drawn coordinates stay whole). A text clear of
    /// the title keeps its place.
    pub(super) fn clear_title(&mut self, pos: Point, title: &Bounds) {
        let extent = self.extent(Side::North, pos);
        if extent.overlaps(title) {
            let near = extent.origin.y + extent.height;
            self.lift += (near - (title.origin.y - PIN_TITLE_GAP)).ceil();
        }
    }

    /// What `port` of `owner` is annotated with: its pins (`2`, or `2,3`
    /// for a ganged port), after `<designator>:` when `owner`'s component
    /// declares more than one connector — counting every connector, not
    /// only those the view shows, so a bare number always means the
    /// component's only connector. A port on no connector (a bare-port
    /// pin, which `check` warns about) shows its pins alone. `None` for a
    /// port with no pins.
    pub(super) fn text(owner: &Instance, port: &Port) -> Option<String> {
        let pins = Pin::display_list(&port.pins)?;
        Some(match &port.connector {
            Some(connector) if owner.connectors.len() > 1 => format!("{}:{pins}", connector.name),
            _ => pins,
        })
    }

    /// The orientation of every annotation on one `side`, whose ports sit
    /// `spacing` apart (at least `MIN_PORT_PITCH`; infinite for a lone
    /// port). West and east texts
    /// stack, so only their height meets a neighbour, and that fits the
    /// pitch whatever their length. North and south texts sit side by
    /// side: they stay level while each ends clear of the next port's
    /// wire (`PIN_NEIGHBOUR_GAP`), and otherwise all turn to run along
    /// their stubs, where again only their height meets a neighbour. One
    /// orientation per side keeps a side's texts parallel.
    pub(super) fn orientation<'a>(
        side: Side,
        texts: impl IntoIterator<Item = &'a str>,
        spacing: f64,
    ) -> PinOrientation {
        let fits = |text: &str| PIN_INSET + Self::width(text) <= spacing - PIN_NEIGHBOUR_GAP;
        match side {
            Side::West | Side::East => PinOrientation::Horizontal,
            Side::North | Side::South => {
                if texts.into_iter().all(fits) {
                    PinOrientation::Horizontal
                } else {
                    PinOrientation::Vertical
                }
            }
        }
    }

    /// The text's estimated length along its baseline.
    fn width(text: &str) -> f64 {
        text.chars().count() as f64 * PIN_CHAR_WIDTH
    }

    /// Where the text is anchored for a port at `pos` labelled as sitting
    /// on `side`, outside the box. Level text sits above a west or east
    /// stub and to the right of a north or south one. Turned text ends
    /// `PIN_INSET` out along the stub, reading toward the port, on the
    /// stub's right on a north side and its left on a south side, so the
    /// two annotations of a short vertical wire between a box's south
    /// port and another's north port sit on opposite sides of it. A north
    /// text sits `lift` farther out.
    pub(super) fn placement(&self, side: Side, pos: Point) -> LabelPlacement {
        let level = |x: f64, y: f64, anchor: &'static str| LabelPlacement {
            x,
            y,
            anchor,
            rotate: 0.0,
            baseline: None,
        };
        let turned = |x: f64, y: f64, rotate: f64| LabelPlacement {
            x,
            y,
            anchor: "end",
            rotate,
            baseline: Some("central"),
        };
        match (self.orientation, side) {
            (_, Side::West) => level(pos.x - PIN_INSET, pos.y - 5.0, "end"),
            (_, Side::East) => level(pos.x + PIN_INSET, pos.y - 5.0, "start"),
            (PinOrientation::Horizontal, Side::North) => {
                level(pos.x + PIN_INSET, pos.y - 5.0 - self.lift, "start")
            }
            (PinOrientation::Horizontal, Side::South) => {
                level(pos.x + PIN_INSET, pos.y + 13.0, "start")
            }
            (PinOrientation::Vertical, Side::North) => {
                turned(pos.x + PIN_ACROSS, pos.y - PIN_INSET - self.lift, 90.0)
            }
            (PinOrientation::Vertical, Side::South) => {
                turned(pos.x - PIN_ACROSS, pos.y + PIN_INSET, -90.0)
            }
        }
    }

    /// The area the text covers when drawn at [`Self::placement`].
    pub(super) fn extent(&self, side: Side, pos: Point) -> Bounds {
        let at = self.placement(side, pos);
        let width = Self::width(&self.text);
        // Along the baseline (u) and across it (v, negative above), before
        // the rotation about the anchor.
        let (u0, u1) = match at.anchor {
            "end" => (-width, 0.0),
            _ => (0.0, width),
        };
        let (v0, v1) = match at.baseline {
            Some("central") => (-FONT_SIZE / 2.0, FONT_SIZE / 2.0),
            _ => (-ASCENT, DESCENT),
        };
        // SVG rotates clockwise on screen: -90° turns the baseline upward
        // and "above" leftward, +90° the reverse.
        let ((x0, x1), (y0, y1)) = if at.rotate < 0.0 {
            ((v0, v1), (-u1, -u0))
        } else if at.rotate > 0.0 {
            ((-v1, -v0), (u0, u1))
        } else {
            ((u0, u1), (v0, v1))
        };
        Bounds {
            origin: Point::new(at.x + x0, at.y + y0),
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{InstanceName, PortName};
    use crate::render::schematic::tests::design_from;
    use crate::render::schematic::{MIN_PORT_PITCH, PIN_TITLE_GAP};

    /// An instance with an HV and an interlock connector (each with a pin
    /// 1 and 2, one of them ganged), a bare-port pin and a pinless port,
    /// beside a single-connector one.
    fn design() -> crate::dsl::ir::Design {
        design_from(
            r#"
component sys {
    msd: Msd;
    lamp: Lamp;
    component Msd {
        connector hv "HV 3p" {
            pub port hv_in "HV IN" pin 1;
            pub port hv_out "HV OUT" pins [2, 3];
        }
        connector intlk "Interlock 2p" {
            pub port intlk_in "INTLK IN" pin 1;
            pub port intlk_out "INTLK OUT" pin 2;
        }
        pub port sense "SENSE" pin 9;
        pub port spare "SPARE";
    }
    component Lamp {
        connector x1 "Lamp 2p" {
            pub port pwr "PWR" pin 1;
            pub port gnd "GND" pins [2, 3];
        }
    }
}
"#,
        )
    }

    fn text_of(design: &crate::dsl::ir::Design, instance: &str, port: &str) -> Option<String> {
        let root = design.get(&design.root).expect("root");
        let owner = design
            .get(&root.children[&InstanceName::from(instance)])
            .expect("child");
        PinAnnotation::text(owner, &owner.ports[&PortName::from(port)])
    }

    #[test]
    fn pins_of_a_component_with_several_connectors_name_their_connector() {
        let design = design();
        assert_eq!(text_of(&design, "msd", "hv_in").as_deref(), Some("hv:1"));
        assert_eq!(
            text_of(&design, "msd", "intlk_in").as_deref(),
            Some("intlk:1")
        );
        assert_eq!(text_of(&design, "msd", "hv_out").as_deref(), Some("hv:2,3"));
    }

    #[test]
    fn pins_of_a_single_connector_component_stay_bare() {
        let design = design();
        assert_eq!(text_of(&design, "lamp", "pwr").as_deref(), Some("1"));
        assert_eq!(text_of(&design, "lamp", "gnd").as_deref(), Some("2,3"));
    }

    #[test]
    fn a_pin_on_no_connector_stays_bare_and_a_pinless_port_has_none() {
        let design = design();
        assert_eq!(text_of(&design, "msd", "sense").as_deref(), Some("9"));
        assert_eq!(text_of(&design, "msd", "spare"), None);
    }

    #[test]
    fn west_and_east_annotations_stay_level_however_long() {
        let long = "bms_signals:2,3,4";
        for side in [Side::West, Side::East] {
            assert_eq!(
                PinAnnotation::orientation(side, [long], MIN_PORT_PITCH),
                PinOrientation::Horizontal
            );
        }
    }

    #[test]
    fn north_and_south_annotations_turn_when_one_would_reach_the_next_port() {
        let pitch = 30.0;
        for side in [Side::North, Side::South] {
            assert_eq!(
                PinAnnotation::orientation(side, ["1", "12", "2,3"], pitch),
                PinOrientation::Horizontal
            );
            assert_eq!(
                PinAnnotation::orientation(side, ["1", "hv:2"], pitch),
                PinOrientation::Vertical
            );
        }
        // At the minimum pitch only a single character stays level.
        assert_eq!(
            PinAnnotation::orientation(Side::North, ["7"], MIN_PORT_PITCH),
            PinOrientation::Horizontal
        );
        assert_eq!(
            PinAnnotation::orientation(Side::North, ["12"], MIN_PORT_PITCH),
            PinOrientation::Vertical
        );
    }

    fn annotation(text: &str, orientation: PinOrientation) -> PinAnnotation {
        PinAnnotation::new(text.to_string(), orientation)
    }

    /// The extent's span across the stub of a port at the origin: y for a
    /// west or east port, x for a north or south one.
    fn across(extent: &Bounds, side: Side) -> (f64, f64) {
        match side {
            Side::West | Side::East => (extent.origin.y, extent.origin.y + extent.height),
            Side::North | Side::South => (extent.origin.x, extent.origin.x + extent.width),
        }
    }

    #[test]
    fn a_long_annotation_clears_the_next_port_at_the_minimum_pitch() {
        // However long the text, the band it covers across the stub stays
        // within one pitch of its own port: it never reaches the next
        // port's line on either side.
        let long = "bms_signals:2,3,4";
        for (side, orientation) in [
            (Side::West, PinOrientation::Horizontal),
            (Side::East, PinOrientation::Horizontal),
            (Side::North, PinOrientation::Vertical),
            (Side::South, PinOrientation::Vertical),
        ] {
            let extent = annotation(long, orientation).extent(side, Point::ORIGIN);
            let (from, to) = across(&extent, side);
            assert!(
                -MIN_PORT_PITCH < from && to < MIN_PORT_PITCH,
                "{side:?}: {from}..{to}"
            );
            // It sits wholly to one side of its own stub.
            assert!(to <= 0.0 || from >= 0.0, "{side:?}: {from}..{to}");
        }
    }

    #[test]
    fn a_turned_annotation_runs_out_along_its_stub_from_the_box() {
        let text = annotation("intlk:1", PinOrientation::Vertical);
        let width = 7.0 * PIN_CHAR_WIDTH;

        let north = text.extent(Side::North, Point::ORIGIN);
        assert_eq!(north.origin.y + north.height, -PIN_INSET);
        assert_eq!(north.height, width);
        assert!(north.origin.x > 0.0, "right of the stub: {north:?}");

        let south = text.extent(Side::South, Point::ORIGIN);
        assert_eq!(south.origin.y, PIN_INSET);
        assert_eq!(south.height, width);
        assert!(
            south.origin.x + south.width < 0.0,
            "left of the stub: {south:?}"
        );
    }

    /// A title band across the top of a box whose top edge is at y = 0,
    /// from `x0` to `x1`.
    fn title(x0: f64, x1: f64) -> Bounds {
        Bounds {
            origin: Point::new(x0, -20.0),
            width: x1 - x0,
            height: 15.0,
        }
    }

    #[test]
    fn a_north_text_meeting_the_title_moves_out_past_it_keeping_its_number_nearest() {
        for orientation in [PinOrientation::Vertical, PinOrientation::Horizontal] {
            let mut text = annotation("iso_up:1", orientation);
            let title = title(-40.0, 40.0);
            assert!(text.extent(Side::North, Point::ORIGIN).overlaps(&title));

            text.clear_title(Point::ORIGIN, &title);
            let moved = text.extent(Side::North, Point::ORIGIN);
            assert!(!moved.overlaps(&title), "{orientation:?}: {moved:?}");
            // Its near end sits just above the title, not farther.
            let near = moved.origin.y + moved.height;
            assert!(near <= title.origin.y - PIN_TITLE_GAP, "{orientation:?}");
            assert!(
                near > title.origin.y - PIN_TITLE_GAP - 1.0,
                "{orientation:?}"
            );
        }
        // A turned text still ends, number last, at its anchor: the end
        // nearest the port.
        let mut text = annotation("iso_up:1", PinOrientation::Vertical);
        text.clear_title(Point::ORIGIN, &title(-40.0, 40.0));
        let at = text.placement(Side::North, Point::ORIGIN);
        let moved = text.extent(Side::North, Point::ORIGIN);
        assert_eq!((at.anchor, at.rotate), ("end", 90.0));
        assert_eq!(at.y, moved.origin.y + moved.height);
    }

    #[test]
    fn a_north_text_clear_of_the_title_keeps_its_place() {
        let mut text = annotation("iso_up:1", PinOrientation::Vertical);
        let before = text.extent(Side::North, Point::ORIGIN);
        // The text runs right of the stub; a title ending left of it.
        text.clear_title(Point::ORIGIN, &title(-60.0, 0.0));
        assert_eq!(text.extent(Side::North, Point::ORIGIN), before);
    }

    #[test]
    fn a_turned_annotation_reads_toward_its_port_so_the_pin_number_ends_beside_it() {
        let text = annotation("intlk:1", PinOrientation::Vertical);
        // The text ends at its anchor, one inset out from the port, and
        // reads toward the port: downward (+90°) above a north port,
        // upward (-90°) below a south one.
        for (side, rotate, end_y) in [
            (Side::North, 90.0, -PIN_INSET),
            (Side::South, -90.0, PIN_INSET),
        ] {
            let at = text.placement(side, Point::ORIGIN);
            assert_eq!(at.anchor, "end", "{side:?}");
            assert_eq!(at.rotate, rotate, "{side:?}");
            assert_eq!(at.y, end_y, "{side:?}");
        }
    }

    #[test]
    fn a_level_annotation_runs_away_from_the_box() {
        let text = annotation("hv:1", PinOrientation::Horizontal);
        let west = text.extent(Side::West, Point::ORIGIN);
        assert_eq!(west.origin.x + west.width, -PIN_INSET);
        let east = text.extent(Side::East, Point::ORIGIN);
        assert_eq!(east.origin.x, PIN_INSET);
    }
}
