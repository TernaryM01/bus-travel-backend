/// Calculate distance between two coordinates using Haversine formula
/// Returns distance in kilometers
pub fn haversine_distance(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lng = (lng2 - lng1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lng / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    EARTH_RADIUS_KM * c
}

/// Check if a pickup point is within the allowed radius of a city center
pub fn is_within_radius(
    pickup_lat: f64,
    pickup_lng: f64,
    center_lat: f64,
    center_lng: f64,
    max_radius_km: f64,
) -> bool {
    haversine_distance(pickup_lat, pickup_lng, center_lat, center_lng) <= max_radius_km
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine_jakarta_bandung() {
        // Jakarta center
        let jakarta = (-6.2088, 106.8456);
        // Bandung center
        let bandung = (-6.9175, 107.6191);

        let distance = haversine_distance(jakarta.0, jakarta.1, bandung.0, bandung.1);
        // Should be approximately 120-130 km
        assert!(distance > 100.0 && distance < 150.0);
    }

    #[test]
    fn test_within_radius() {
        let center = (-6.2088, 106.8456); // Jakarta
        let nearby = (-6.21, 106.85);     // Very close to center

        assert!(is_within_radius(nearby.0, nearby.1, center.0, center.1, 10.0));

        let far = (-6.9175, 107.6191);    // Bandung
        assert!(!is_within_radius(far.0, far.1, center.0, center.1, 10.0));
    }
}
