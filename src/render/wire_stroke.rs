//! How a wire is stroked in its colour, shared by every renderer that draws
//! wires: a dark casing under a core in the base colour, plus a dashed
//! tracer stripe for a two-tone wire.
//!
//! The paint rides on each element as a `stroke` presentation attribute,
//! and the authored colour as `data-color`. Widths, dashes and the casing's
//! own paint live in each renderer's stylesheet, under the classes its
//! [`WireLayers`] names. A presentation attribute loses to any stylesheet
//! rule, so an embed host's own `stroke` rule still wins over the colour.

use svg::Node;

use crate::dsl::ir::WireColor;

/// The classes one renderer tags the three layers of a wire with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WireLayers {
    /// The wider dark stroke under the core, so a light core reads on any
    /// background.
    pub(crate) casing: &'static str,
    /// The wire itself, in its base colour.
    pub(crate) core: &'static str,
    /// The stripe of a two-tone wire, over the core.
    pub(crate) tracer: &'static str,
}

impl WireLayers {
    /// How a wire of `color` is stroked under these classes.
    pub(crate) fn stroke(self, color: &WireColor) -> WireStroke<'_> {
        WireStroke {
            color,
            layers: self,
        }
    }
}

/// One wire's stroke: its colour, drawn as the layers a [`WireLayers`]
/// names. Each method takes the bare shape (a path, a polyline) and
/// returns it dressed as that layer.
pub(crate) struct WireStroke<'a> {
    color: &'a WireColor,
    layers: WireLayers,
}

impl WireStroke<'_> {
    /// The paint the core is stroked with: the base colour. Every IEC 60757
    /// name is a CSS colour name; a name outside the set passes through
    /// verbatim, so an unknown name that is no CSS colour leaves the core
    /// unpainted and the wire reads as its casing alone.
    fn paint(&self) -> &str {
        self.color.css()
    }

    /// The paint of a two-tone wire's tracer stripe.
    fn tracer_paint(&self) -> Option<&str> {
        self.color.tracer()
    }

    /// `shape` as the casing. It carries no paint of its own: the
    /// stylesheet strokes it dark and wider than the core.
    #[must_use]
    pub(crate) fn casing<T: Node>(&self, mut shape: T) -> T {
        shape.assign("class", self.layers.casing);
        shape
    }

    /// `shape` as the core, stroked in the base colour and carrying the
    /// authored colour as `data-color` for host stylesheets.
    #[must_use]
    pub(crate) fn core<T: Node>(&self, mut shape: T) -> T {
        shape.assign("class", self.layers.core);
        shape.assign("stroke", self.paint());
        shape.assign("data-color", self.color.to_string());
        shape
    }

    /// `shape` as the tracer stripe, stroked in the tracer colour (the
    /// dash comes from the stylesheet), or `None` for a single-colour wire.
    #[must_use]
    pub(crate) fn tracer<T: Node>(&self, mut shape: T) -> Option<T> {
        let paint = self.tracer_paint()?;
        shape.assign("class", self.layers.tracer);
        shape.assign("stroke", paint);
        shape.assign("data-color", self.color.to_string());
        Some(shape)
    }

    /// Every layer of a wire drawn as one shape, in draw order: casing,
    /// core, then the tracer when the wire has one.
    pub(crate) fn layers<T: Node + Clone>(&self, shape: T) -> impl Iterator<Item = T> + use<T> {
        // `Node` brings its own boxing `clone`; this is the element's.
        let tracer = self.tracer(Clone::clone(&shape));
        [self.casing(Clone::clone(&shape)), self.core(shape)]
            .into_iter()
            .chain(tracer)
    }
}

#[cfg(test)]
mod tests {
    use svg::node::element::Path;

    use super::*;

    const LAYERS: WireLayers = WireLayers {
        casing: "w-casing",
        core: "w",
        tracer: "w-tracer",
    };

    fn shape() -> Path {
        Path::new().set("d", "M0,0 L10,0")
    }

    #[test]
    fn core_is_stroked_in_the_base_colour_and_tagged_with_it() {
        let color = WireColor::from("red");
        let core = LAYERS.stroke(&color).core(shape()).to_string();
        assert!(core.contains("class=\"w\""), "{core}");
        assert!(core.contains("stroke=\"red\""), "{core}");
        assert!(core.contains("data-color=\"red\""), "{core}");
    }

    #[test]
    fn casing_carries_its_class_and_no_paint() {
        let color = WireColor::from("white");
        let casing = LAYERS.stroke(&color).casing(shape()).to_string();
        assert!(casing.contains("class=\"w-casing\""), "{casing}");
        assert!(!casing.contains("stroke="), "{casing}");
        assert!(!casing.contains("data-color="), "{casing}");
    }

    #[test]
    fn two_tone_wire_layers_casing_core_then_tracer() {
        let color = WireColor::from("green/yellow");
        let layers: Vec<String> = LAYERS
            .stroke(&color)
            .layers(shape())
            .map(|layer| layer.to_string())
            .collect();
        assert_eq!(layers.len(), 3);
        assert!(layers[0].contains("class=\"w-casing\""));
        assert!(layers[1].contains("class=\"w\"") && layers[1].contains("stroke=\"green\""));
        assert!(
            layers[2].contains("class=\"w-tracer\"") && layers[2].contains("stroke=\"yellow\"")
        );
        assert!(layers[2].contains("data-color=\"green/yellow\""));
    }

    #[test]
    fn single_colour_wire_has_no_tracer() {
        let color = WireColor::from("blue");
        let stroke = LAYERS.stroke(&color);
        assert!(stroke.tracer(shape()).is_none());
        assert_eq!(stroke.layers(shape()).count(), 2);
    }

    #[test]
    fn synonym_paints_and_tags_as_its_canonical_name() {
        let color = WireColor::from("purple");
        let core = LAYERS.stroke(&color).core(shape()).to_string();
        assert!(core.contains("stroke=\"violet\""), "{core}");
        assert!(core.contains("data-color=\"violet\""), "{core}");
    }

    #[test]
    fn unknown_colour_paints_verbatim() {
        let color = WireColor::from("chartreuse");
        let core = LAYERS.stroke(&color).core(shape()).to_string();
        assert!(core.contains("stroke=\"chartreuse\""), "{core}");
        assert!(core.contains("data-color=\"chartreuse\""), "{core}");
    }
}
