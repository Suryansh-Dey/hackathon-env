//! Locations and H3 cells (PLAN.md §12.5).
//!
//! Resolution 8 is used for matching, resolution 7 for feeds and display.

use crate::DomainError;
use h3o::{CellIndex, Resolution};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub const MATCH_RES: Resolution = Resolution::Eight;
pub const FEED_RES: Resolution = Resolution::Seven;

/// WGS84 point.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

impl LatLng {
    pub fn new(lat: f64, lng: f64) -> Result<Self, DomainError> {
        let p = LatLng { lat, lng };
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if !(self.lat.is_finite() && self.lng.is_finite()) || !(-90.0..=90.0).contains(&self.lat) || !(-180.0..=180.0).contains(&self.lng) {
            return Err(DomainError::validation("lat/lng out of range"));
        }
        Ok(())
    }

    fn h3(&self) -> Result<h3o::LatLng, DomainError> {
        h3o::LatLng::new(self.lat, self.lng).map_err(|e| DomainError::validation(format!("bad coordinates: {e}")))
    }

    /// H3 cell index string at the given resolution.
    pub fn cell(&self, res: Resolution) -> Result<String, DomainError> {
        Ok(self.h3()?.to_cell(res).to_string())
    }

    pub fn cell_r8(&self) -> Result<String, DomainError> {
        self.cell(MATCH_RES)
    }

    pub fn cell_r7(&self) -> Result<String, DomainError> {
        self.cell(FEED_RES)
    }

    /// Great-circle distance in metres (haversine).
    pub fn distance_m(&self, other: &LatLng) -> f64 {
        const R: f64 = 6_371_008.8;
        let (la1, la2) = (self.lat.to_radians(), other.lat.to_radians());
        let dla = (other.lat - self.lat).to_radians();
        let dlo = (other.lng - self.lng).to_radians();
        let a = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
        2.0 * R * a.sqrt().asin()
    }
}

fn parse_cell(cell: &str) -> Result<CellIndex, DomainError> {
    CellIndex::from_str(cell).map_err(|_| DomainError::validation(format!("invalid H3 cell {cell:?}")))
}

/// Validates an H3 cell string and returns its resolution.
pub fn cell_resolution(cell: &str) -> Result<u8, DomainError> {
    Ok(u8::from(parse_cell(cell)?.resolution()))
}

/// Centre point of an H3 cell (the approximate location shown to vendors).
pub fn cell_center(cell: &str) -> Result<LatLng, DomainError> {
    let ll = h3o::LatLng::from(parse_cell(cell)?);
    Ok(LatLng { lat: ll.lat(), lng: ll.lng() })
}

/// Parent of a cell at a coarser resolution.
pub fn cell_parent(cell: &str, res: Resolution) -> Result<String, DomainError> {
    parse_cell(cell)?.parent(res).map(|c| c.to_string()).ok_or_else(|| DomainError::validation("parent resolution must be coarser"))
}

/// All cells within `k` rings of `cell`, including `cell` (PLAN.md §12.7 `ring(h3_r8, k)`).
pub fn grid_disk(cell: &str, k: u32) -> Result<Vec<String>, DomainError> {
    let c = parse_cell(cell)?;
    let mut cells: Vec<String> = c.grid_disk::<Vec<_>>(k).into_iter().map(|c| c.to_string()).collect();
    cells.sort();
    Ok(cells)
}

/// Cells of exactly ring `k` (used by `widen_search`).
pub fn grid_ring(cell: &str, k: u32) -> Result<Vec<String>, DomainError> {
    let inner: std::collections::BTreeSet<String> = if k == 0 { Default::default() } else { grid_disk(cell, k - 1)?.into_iter().collect() };
    Ok(grid_disk(cell, k)?.into_iter().filter(|c| !inner.contains(c)).collect())
}

/// Resolution-8 cells whose centres fall within a circle.
pub fn cells_within_radius(center: &LatLng, radius_m: f64) -> Result<Vec<String>, DomainError> {
    let origin = center.cell_r8()?;
    // Average r8 centre spacing is ~0.8 km; over-cover and filter.
    let k = ((radius_m / 750.0).ceil() as u32).saturating_add(1).min(200);
    Ok(grid_disk(&origin, k)?.into_iter().filter(|c| cell_center(c).map(|p| p.distance_m(center) <= radius_m).unwrap_or(false)).collect())
}

/// Ray-casting point-in-polygon on lat/lng (adequate for city-scale service areas).
pub fn point_in_polygon(p: &LatLng, poly: &[LatLng]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.lat > p.lat) != (b.lat > p.lat) {
            let x = (b.lng - a.lng) * (p.lat - a.lat) / (b.lat - a.lat) + a.lng;
            if p.lng < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Resolution-8 cells whose centres fall within a polygon.
pub fn cells_within_polygon(poly: &[LatLng]) -> Result<Vec<String>, DomainError> {
    if poly.len() < 3 {
        return Err(DomainError::validation("polygon needs at least 3 points"));
    }
    let n = poly.len() as f64;
    let centroid = LatLng { lat: poly.iter().map(|p| p.lat).sum::<f64>() / n, lng: poly.iter().map(|p| p.lng).sum::<f64>() / n };
    let max_r = poly.iter().map(|p| p.distance_m(&centroid)).fold(0.0, f64::max);
    Ok(cells_within_radius(&centroid, max_r + 1000.0)?
        .into_iter()
        .filter(|c| cell_center(c).map(|p| point_in_polygon(&p, poly)).unwrap_or(false))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELHI: LatLng = LatLng { lat: 28.6139, lng: 77.2090 };

    #[test]
    fn cells_have_expected_resolution() {
        assert_eq!(cell_resolution(&DELHI.cell_r8().unwrap()).unwrap(), 8);
        assert_eq!(cell_resolution(&DELHI.cell_r7().unwrap()).unwrap(), 7);
        assert_eq!(cell_parent(&DELHI.cell_r8().unwrap(), FEED_RES).unwrap(), DELHI.cell_r7().unwrap());
    }

    #[test]
    fn disk_and_ring_sizes() {
        let c = DELHI.cell_r8().unwrap();
        assert_eq!(grid_disk(&c, 1).unwrap().len(), 7);
        assert_eq!(grid_ring(&c, 1).unwrap().len(), 6);
        assert_eq!(grid_ring(&c, 0).unwrap(), vec![c]);
    }

    #[test]
    fn radius_cover_contains_center_and_respects_radius() {
        let cells = cells_within_radius(&DELHI, 3000.0).unwrap();
        assert!(cells.contains(&DELHI.cell_r8().unwrap()));
        assert!(cells.len() > 10);
        for c in cells {
            assert!(cell_center(&c).unwrap().distance_m(&DELHI) <= 3000.0);
        }
    }

    #[test]
    fn polygon_contains() {
        let sq = [
            LatLng { lat: 28.60, lng: 77.20 },
            LatLng { lat: 28.60, lng: 77.22 },
            LatLng { lat: 28.62, lng: 77.22 },
            LatLng { lat: 28.62, lng: 77.20 },
        ];
        assert!(point_in_polygon(&DELHI, &sq));
        assert!(!point_in_polygon(&LatLng { lat: 28.7, lng: 77.2 }, &sq));
        assert!(!cells_within_polygon(&sq).unwrap().is_empty());
    }

    #[test]
    fn haversine_is_sane() {
        let mumbai = LatLng { lat: 19.0760, lng: 72.8777 };
        let d = DELHI.distance_m(&mumbai) / 1000.0;
        assert!((1140.0..1160.0).contains(&d), "{d}");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(LatLng::new(91.0, 0.0).is_err());
        assert!(cell_center("not-a-cell").is_err());
    }
}
