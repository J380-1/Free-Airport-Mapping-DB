use serde::{Deserialize, Serialize};

/// Geometry kind mandated by DO-272 for each feature type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeomKind {
    Point,
    Curve,
    Surface,
}

macro_rules! layers {
    ($( $variant:ident => $name:literal, $kind:ident ),* $(,)?) => {
        /// Every AMXM 2.0 feature type. Variant order is the canonical output order.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum Layer { $( $variant ),* }

        pub const ALL_LAYERS: &[Layer] = &[ $( Layer::$variant ),* ];

        impl Layer {
            /// Lowercase AMXM feature name, used as the file stem and Navigraph layer id.
            pub fn name(self) -> &'static str {
                match self { $( Layer::$variant => $name ),* }
            }
            pub fn kind(self) -> GeomKind {
                match self { $( Layer::$variant => GeomKind::$kind ),* }
            }
            pub fn from_name(s: &str) -> Option<Layer> {
                let s = s.to_ascii_lowercase();
                ALL_LAYERS.iter().copied().find(|l| l.name() == s)
            }
        }
    };
}

layers! {
    AtcBlindSpot => "atcblindspot", Surface,
    AerodromeReferencePoint => "aerodromereferencepoint", Point,
    AerodromeSign => "aerodromesign", Point,
    AerodromeSurfaceLighting => "aerodromesurfacelighting", Point,
    ApronElement => "apronelement", Surface,
    ArrestingGearLocation => "arrestinggearlocation", Curve,
    ArrestingSystemLocation => "arrestingsystemlocation", Surface,
    AsrnEdge => "asrnedge", Curve,
    AsrnNode => "asrnnode", Point,
    Blastpad => "blastpad", Surface,
    BridgeSide => "bridgeside", Curve,
    ConstructionArea => "constructionarea", Surface,
    DeicingArea => "deicingarea", Surface,
    DeicingGroup => "deicinggroup", Surface,
    FinalApproachAndTakeOffArea => "finalapproachandtakeoffarea", Surface,
    FrequencyArea => "frequencyarea", Surface,
    HelipadThreshold => "helipadthreshold", Point,
    Hotspot => "hotspot", Surface,
    LandAndHoldShortOperationLocation => "landandholdshortoperationlocation", Curve,
    PaintedCenterline => "paintedcenterline", Curve,
    ParkingStandArea => "parkingstandarea", Surface,
    ParkingStandLocation => "parkingstandlocation", Point,
    PositionMarking => "positionmarking", Point,
    RunwayCenterlinePoint => "runwaycenterlinepoint", Point,
    RunwayDisplacedArea => "runwaydisplacedarea", Surface,
    RunwayElement => "runwayelement", Surface,
    RunwayExitLine => "runwayexitline", Curve,
    RunwayIntersection => "runwayintersection", Surface,
    RunwayMarking => "runwaymarking", Surface,
    RunwayShoulder => "runwayshoulder", Surface,
    RunwayThreshold => "runwaythreshold", Point,
    ServiceRoad => "serviceroad", Surface,
    StandGuidanceLine => "standguidanceline", Curve,
    Stopway => "stopway", Surface,
    SurveyControlPoint => "surveycontrolpoint", Point,
    TaxiwayElement => "taxiwayelement", Surface,
    TaxiwayGuidanceLine => "taxiwayguidanceline", Curve,
    TaxiwayHoldingPosition => "taxiwayholdingposition", Curve,
    TaxiwayIntersectionMarking => "taxiwayintersectionmarking", Curve,
    TaxiwayShoulder => "taxiwayshoulder", Surface,
    TouchDownLiftOffArea => "touchdownliftoffarea", Surface,
    VerticalLineStructure => "verticallinestructure", Curve,
    VerticalPointStructure => "verticalpointstructure", Point,
    VerticalPolygonalStructure => "verticalpolygonalstructure", Surface,
    Water => "water", Surface,
}

/// Layers a moving map actually draws. Everything else is still generated with
/// `--profile full` (the default).
pub const MAP_PROFILE: &[Layer] = &[
    Layer::AerodromeReferencePoint,
    Layer::RunwayElement,
    Layer::RunwayThreshold,
    Layer::RunwayDisplacedArea,
    Layer::RunwayIntersection,
    Layer::RunwayShoulder,
    Layer::Blastpad,
    Layer::Stopway,
    Layer::RunwayMarking,
    Layer::PaintedCenterline,
    Layer::RunwayExitLine,
    Layer::TaxiwayElement,
    Layer::TaxiwayShoulder,
    Layer::TaxiwayGuidanceLine,
    Layer::TaxiwayHoldingPosition,
    Layer::TaxiwayIntersectionMarking,
    Layer::StandGuidanceLine,
    Layer::ApronElement,
    Layer::ParkingStandLocation,
    Layer::ParkingStandArea,
    Layer::AsrnNode,
    Layer::AsrnEdge,
    Layer::AerodromeSign,
    Layer::VerticalPolygonalStructure,
    Layer::ServiceRoad,
    Layer::Water,
    Layer::ConstructionArea,
    Layer::DeicingArea,
    Layer::BridgeSide,
    Layer::Hotspot,
    Layer::FrequencyArea,
    Layer::FinalApproachAndTakeOffArea,
    Layer::TouchDownLiftOffArea,
    Layer::HelipadThreshold,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_all_45_layers_with_unique_names() {
        assert_eq!(ALL_LAYERS.len(), 45);
        let mut names: Vec<_> = ALL_LAYERS.iter().map(|l| l.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 45);
        assert_eq!(Layer::from_name("RunwayElement"), Some(Layer::RunwayElement));
        assert_eq!(Layer::TaxiwayElement.kind(), GeomKind::Surface);
        assert_eq!(Layer::AsrnEdge.kind(), GeomKind::Curve);
    }
}
