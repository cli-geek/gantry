use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A geocoded place. Coordinates come from bundled GeoNames data; no
/// geocoding service is ever called (§12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Place {
    pub name: String,
    /// GeoNames admin1 code: the state code in the US ("CA"), a numeric or
    /// alphanumeric region code elsewhere.
    pub admin1: Option<String>,
    /// ISO 3166-1 alpha-2.
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DistanceUnit {
    Mi,
    Km,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Distance {
    pub value: f64,
    pub unit: DistanceUnit,
}

impl Distance {
    pub fn km(self) -> f64 {
        match self.unit {
            DistanceUnit::Km => self.value,
            DistanceUnit::Mi => self.value * 1.609_344,
        }
    }
}

/// Great-circle distance in kilometres. Straight-line distance is the v1
/// commute measure (§7.2).
pub fn haversine_km(a: &Place, b: &Place) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0088;
    let (lat1, lat2) = (a.lat.to_radians(), b.lat.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(lat: f64, lon: f64) -> Place {
        Place {
            name: String::new(),
            admin1: None,
            country: "US".into(),
            lat,
            lon,
        }
    }

    #[test]
    fn haversine_sf_to_nyc() {
        let d = haversine_km(&place(37.7749, -122.4194), &place(40.7128, -74.0060));
        assert!((d - 4129.0).abs() < 10.0, "{d}");
    }

    #[test]
    fn miles_convert_to_km() {
        let d = Distance {
            value: 10.0,
            unit: DistanceUnit::Mi,
        };
        assert!((d.km() - 16.093).abs() < 0.001);
    }
}
