// Table 7's sky map: a catalog of 1,500 real stars, six constellations, and the
// view that wanders across the sky, flies to a constellation and projects the
// visible stars onto the circle in the middle of the table. Ported from the
// original's Stars.java and the StarState class in Field7Delegate.java.
//
// Everything runs in f64 and sim seconds, like the original's doubles and
// game-time nanoseconds.

use std::collections::{BTreeSet, HashMap};
use std::f64::consts::TAU;
use std::sync::OnceLock;

/// How wide a view a constellation gets, as a multiple of its angular radius.
const CONSTELLATION_RADIUS_MULTIPLIER: f64 = 1.2;
/// Seconds for a full sweep across the sky while wandering.
const WANDER_PERIOD: f64 = 40.0;
/// Seconds to fly to a constellation or back to the start.
const ANIMATION_SECONDS: f64 = 10.0;
/// Half the angle (radians) of sky shown while wandering.
const WANDER_RADIUS: f64 = 0.4;

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

pub struct Catalog {
    pub dec: Vec<f64>,
    pub ra: Vec<f64>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub magnitude: Vec<f64>,
}

impl Catalog {
    pub fn len(&self) -> usize { self.x.len() }
}

/// The star catalog, parsed once from `assets/stars.txt` (declination, right
/// ascension in degrees, magnitude), brightest first.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut c = Catalog { dec: vec![], ra: vec![], x: vec![], y: vec![], z: vec![], magnitude: vec![] };
        for line in include_str!("../../assets/stars.txt").lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let v: Vec<f64> = line.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if v.len() != 3 { continue; }
            let (rho, theta) = (v[0].to_radians(), v[1].to_radians());
            c.dec.push(rho);
            c.ra.push(theta);
            c.x.push(rho.cos() * theta.cos());
            c.y.push(-rho.cos() * theta.sin());
            c.z.push(rho.sin());
            c.magnitude.push(v[2]);
        }
        c
    })
}

fn great_circle_distance(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    (dec1.sin() * dec2.sin() + dec1.cos() * dec2.cos() * (ra1 - ra2).cos()).clamp(-1.0, 1.0).acos()
}

// ---------------------------------------------------------------------------
// Constellations
// ---------------------------------------------------------------------------

pub struct Constellation {
    pub name: &'static str,
    pub star_indices: BTreeSet<usize>,
    pub center_ra: f64,
    pub center_dec: f64,
    pub angular_radius: f64,
    /// Maps the smaller index of a segment to the larger indices it joins.
    pub segments_by_index: HashMap<usize, Vec<usize>>,
}

impl Constellation {
    /// `segments` is flattened pairs of star indices. The centre is the mean of
    /// its stars' right ascension and declination, and the radius the farthest
    /// star from it, like the original.
    fn with_segments(name: &'static str, segments: &[usize]) -> Self {
        let cat = catalog();
        let mut star_indices = BTreeSet::new();
        let mut segments_by_index: HashMap<usize, Vec<usize>> = HashMap::new();
        for pair in segments.chunks(2) {
            let (a, b) = (pair[0], pair[1]);
            let list = segments_by_index.entry(a.min(b)).or_default();
            if !list.contains(&a.max(b)) { list.push(a.max(b)); }
            star_indices.insert(a);
            star_indices.insert(b);
        }
        let n = star_indices.len() as f64;
        let avg_ra = star_indices.iter().map(|&i| cat.ra[i]).sum::<f64>() / n;
        let avg_dec = star_indices.iter().map(|&i| cat.dec[i]).sum::<f64>() / n;
        let max_dist = star_indices.iter()
            .map(|&i| great_circle_distance(cat.ra[i], cat.dec[i], avg_ra, avg_dec))
            .fold(0.0, f64::max);
        Constellation { name, star_indices, center_ra: avg_ra, center_dec: avg_dec, angular_radius: max_dist, segments_by_index }
    }
}

/// The six constellations, as in the original (names from the IAU, star indices
/// into the catalog above).
pub fn constellations() -> &'static [Constellation] {
    static LIST: OnceLock<Vec<Constellation>> = OnceLock::new();
    LIST.get_or_init(|| vec![
        Constellation::with_segments("Canis Major", &[
            0, 785, 580, 785, 566, 580, 566, 785, 0, 47, 0, 37, 47, 494, 461, 494, 23, 461,
            23, 37, 37, 90]),
        Constellation::with_segments("Gemini", &[
            17, 555, 317, 555, 401, 555, 401, 817, 326, 817, 45, 817,
            298, 555, 298, 320, 248, 320, 298, 519, 43, 519,
            187, 817, 187, 601, 146, 187]),
        Constellation::with_segments("Leo", &[
            22, 281, 50, 281, 50, 269, 269, 454, 167, 454, 167, 167, 281, 167, 167, 737, 737, 879,
            454, 879, 50, 97, 63, 97, 63, 241, 97, 241, 241, 281, 241, 516, 516, 548]),
        Constellation::with_segments("Orion", &[
            5, 247, 74, 247, 58, 31, 29, 31, 29, 74, 7, 26, 7, 31, 7, 253, 26, 74, 26, 253,
            7, 594, 594, 823, 809, 823, 823, 1053, 809, 1053,
            26, 212, 212, 351, 351, 365, 212, 768, 768, 1052, 551, 1052]),
        Constellation::with_segments("Sagittarius", &[
            53, 240, 101, 240, 53, 205, 101, 205, 35, 101, 35, 117, 35, 168, 117, 168, 35, 190,
            205, 117, 134, 205, 117, 134, 134, 432]),
        Constellation::with_segments("Ursa Major", &[
            36, 82, 82, 86, 36, 233, 86, 233, 233, 33, 33, 73, 38, 73,
            36, 348, 82, 402, 348, 402, 245, 348, 245, 402,
            86, 357, 283, 357, 283, 743, 176, 357, 176, 188, 188, 273,
            204, 402, 204, 315, 200, 315]),
    ])
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProjectionTarget {
    pub right_ascension: f64,
    pub declination: f64,
    pub angular_radius: f64,
}

/// A star in view: its offset from the centre of the view (in radians of sky),
/// its magnitude and its catalog index.
#[derive(Clone, Copy, Debug)]
pub struct ProjectedStar {
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub magnitude: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { Wandering, Constellation }

pub struct StarState {
    pub activated: BTreeSet<usize>,
    /// Indices into `constellations()` that have been locked in.
    pub locked: Vec<usize>,
    /// Index into `constellations()` of the one the view is on (or flying to).
    pub current: Option<usize>,
    pub target: ProjectionTarget,
    pub projection: Vec<ProjectedStar>,
    index_to_projection: Vec<i32>,
    pub mode: Mode,
    wander_seconds: f64,
    animate_from: ProjectionTarget,
    animate_to: ProjectionTarget,
    /// Seconds into the current fly-to; negative when not flying.
    animation_elapsed: f64,
}

fn interp(start: f64, end: f64, fraction: f64) -> f64 { start + fraction * (end - start) }

impl StarState {
    pub fn new() -> Self {
        StarState {
            activated: BTreeSet::new(),
            locked: Vec::new(),
            current: None,
            target: ProjectionTarget::default(),
            projection: Vec::new(),
            index_to_projection: vec![-1; catalog().len()],
            mode: Mode::Wandering,
            wander_seconds: 0.0,
            animate_from: ProjectionTarget::default(),
            animate_to: ProjectionTarget::default(),
            animation_elapsed: -1.0,
        }
    }

    pub fn current_constellation(&self) -> Option<&'static Constellation> {
        self.current.map(|i| &constellations()[i])
    }

    pub fn is_flying(&self) -> bool { self.animation_elapsed >= 0.0 }

    /// Advances the view by `dt` sim seconds and projects the stars it shows.
    pub fn tick(&mut self, dt: f64) {
        if self.animation_elapsed >= 0.0 {
            self.update_animation(dt);
        } else if self.mode == Mode::Wandering {
            self.update_wandering(dt);
        } else if let Some(c) = self.current_constellation() {
            // Pinned to the constellation once the fly-to is over.
            self.target = ProjectionTarget {
                right_ascension: c.center_ra,
                declination: c.center_dec,
                angular_radius: CONSTELLATION_RADIUS_MULTIPLIER * c.angular_radius,
            };
        }
        self.project();
    }

    fn update_wandering(&mut self, dt: f64) {
        self.wander_seconds = (self.wander_seconds + dt) % WANDER_PERIOD;
        // Going from tau down to 0 makes the sky pass left to right.
        self.target = ProjectionTarget {
            right_ascension: TAU * (1.0 - self.wander_seconds / WANDER_PERIOD),
            declination: 0.0,
            angular_radius: WANDER_RADIUS,
        };
    }

    fn update_animation(&mut self, dt: f64) {
        self.animation_elapsed += dt;
        let fraction = (self.animation_elapsed / ANIMATION_SECONDS).min(1.0);
        let (src, dst) = (self.animate_from, self.animate_to);
        self.target = ProjectionTarget {
            right_ascension: interp(src.right_ascension, dst.right_ascension, fraction),
            declination: interp(src.declination, dst.declination, fraction),
            angular_radius: interp(src.angular_radius, dst.angular_radius, fraction),
        };
        if self.animation_elapsed >= ANIMATION_SECONDS {
            self.animation_elapsed = -1.0;
        }
    }

    /// Rotates every star so the view's centre is straight ahead and keeps those
    /// within the view's angular radius, projected orthographically.
    fn project(&mut self) {
        let cat = catalog();
        self.projection.clear();
        self.index_to_projection.iter_mut().for_each(|v| *v = -1);
        let t = self.target;
        let radius_sq = t.angular_radius * t.angular_radius;
        let (sin_ra, cos_ra) = t.right_ascension.sin_cos();
        let (sin_dec, cos_dec) = t.declination.sin_cos();
        for i in 0..cat.len() {
            // Around Z for right ascension, then around Y for declination: the point
            // looked at ends up at (1, 0, 0); Y becomes screen x and Z screen y.
            let x1 = cat.x[i] * cos_ra - cat.y[i] * sin_ra;
            let y1 = cat.x[i] * sin_ra + cat.y[i] * cos_ra;
            let z1 = cat.z[i];
            let x2 = x1 * cos_dec + z1 * sin_dec;
            let y2 = y1;
            let z2 = -x1 * sin_dec + z1 * cos_dec;
            if x2 > 0.0 && y2 * y2 + z2 * z2 < radius_sq {
                self.index_to_projection[i] = self.projection.len() as i32;
                self.projection.push(ProjectedStar { index: i, x: y2, y: z2, magnitude: cat.magnitude[i] });
            }
        }
    }

    /// The projected star for catalog index `star`, if it is in view.
    pub fn projected(&self, star: usize) -> Option<&ProjectedStar> {
        match self.index_to_projection.get(star) {
            Some(&p) if p >= 0 => self.projection.get(p as usize),
            _ => None,
        }
    }

    pub fn all_stars_in_constellation_active(&self, c: &Constellation) -> bool {
        c.star_indices.iter().all(|s| self.activated.contains(s))
    }

    pub fn all_stars_in_current_active(&self) -> bool {
        self.current_constellation().map_or(false, |c| self.all_stars_in_constellation_active(c))
    }

    pub fn all_constellations_locked(&self) -> bool {
        (0..constellations().len()).all(|i| self.locked.contains(&i))
    }

    pub fn lock_current(&mut self) {
        if let Some(i) = self.current {
            if !self.locked.contains(&i) { self.locked.push(i); }
        }
    }

    /// Lights every star of the current constellation within 0.1 of the point
    /// `(x, y)` (the ball, relative to the middle of the star circle and scaled so
    /// the circle has radius 1). Returns how many were newly lit.
    pub fn activate_near(&mut self, x: f64, y: f64) -> usize {
        let Some(c) = self.current_constellation() else { return 0 };
        if x * x + y * y >= 1.0 { return 0; }
        let mut newly = 0;
        for &star in &c.star_indices {
            let Some(p) = self.projected(star).copied() else { continue };
            let (px, py) = (p.x / self.target.angular_radius, p.y / self.target.angular_radius);
            if (x - px).powi(2) + (y - py).powi(2) < 0.1 * 0.1 && self.activated.insert(star) {
                newly += 1;
            }
        }
        newly
    }

    /// Starts the 10 s fly to a constellation.
    pub fn animate_to_constellation(&mut self, index: usize) {
        let c = &constellations()[index];
        self.current = Some(index);
        self.animate_from = self.target;
        self.animate_to = ProjectionTarget {
            right_ascension: c.center_ra,
            declination: c.center_dec,
            angular_radius: CONSTELLATION_RADIUS_MULTIPLIER * c.angular_radius,
        };
        // Take the shorter way round in right ascension.
        if self.animate_from.right_ascension > c.center_ra + TAU / 2.0 {
            self.animate_from.right_ascension -= TAU;
        } else if self.animate_from.right_ascension < c.center_ra - TAU / 2.0 {
            self.animate_from.right_ascension += TAU;
        }
        self.animation_elapsed = 0.0;
    }

    pub fn animate_to_start_position(&mut self) {
        self.current = None;
        self.animate_from = self.target;
        self.animate_to = ProjectionTarget { right_ascension: 0.0, declination: 0.0, angular_radius: WANDER_RADIUS };
        self.animation_elapsed = 0.0;
    }

    pub fn enter_wandering_mode(&mut self) {
        if self.mode != Mode::Wandering {
            self.animate_to_start_position();
            self.mode = Mode::Wandering;
            self.wander_seconds = 0.0;
        }
    }

    pub fn reset_and_wander(&mut self) {
        self.activated.clear();
        self.locked.clear();
        self.enter_wandering_mode();
    }

    /// Flies to a random constellation that is neither current nor locked.
    /// `random` returns a number in [0, 1). False if none is left.
    pub fn switch_to_random_unlocked(&mut self, mut random: impl FnMut() -> f64) -> bool {
        let candidates: Vec<usize> = (0..constellations().len())
            .filter(|&i| Some(i) != self.current && !self.locked.contains(&i))
            .collect();
        if candidates.is_empty() { return false; }
        let pick = candidates[((random() * candidates.len() as f64) as usize).min(candidates.len() - 1)];
        self.animate_to_constellation(pick);
        self.mode = Mode::Constellation;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orion() -> usize { constellations().iter().position(|c| c.name == "Orion").unwrap() }

    #[test]
    fn the_catalog_has_1500_stars_brightest_first() {
        let c = catalog();
        assert_eq!(c.len(), 1500);
        assert!((c.magnitude[0] - -1.43).abs() < 1e-9, "Sirius");
        assert!(c.magnitude.windows(2).all(|w| w[0] <= w[1] + 1e-9), "sorted by brightness");
        let norm = (c.x[10].powi(2) + c.y[10].powi(2) + c.z[10].powi(2)).sqrt();
        assert!((norm - 1.0).abs() < 1e-12, "unit vectors");
    }

    #[test]
    fn there_are_six_constellations_with_sensible_centres_and_radii() {
        let list = constellations();
        let names: Vec<&str> = list.iter().map(|c| c.name).collect();
        assert_eq!(names, ["Canis Major", "Gemini", "Leo", "Orion", "Sagittarius", "Ursa Major"]);
        for c in list {
            assert!(c.star_indices.len() >= 8, "{}: {} stars", c.name, c.star_indices.len());
            assert!(c.angular_radius > 0.05 && c.angular_radius < 1.0, "{}: radius {}", c.name, c.angular_radius);
            assert!(c.star_indices.iter().all(|&s| s < catalog().len()));
            // (Leo's data has one degenerate segment from star 167 to itself; kept as in the original.)
            for (&a, ends) in &c.segments_by_index { assert!(ends.iter().all(|&b| b >= a), "segments run small to large"); }
        }
        let o = &list[orion()];
        assert!(o.center_ra.to_degrees() > 70.0 && o.center_ra.to_degrees() < 95.0, "Orion is near RA 5h: {}", o.center_ra.to_degrees());
        assert!(o.star_indices.contains(&7) && o.star_indices.contains(&26), "Betelgeuse and Rigel are in Orion's list");
    }

    #[test]
    fn looking_at_a_star_puts_it_dead_centre_and_east_is_on_the_left() {
        let c = catalog();
        let mut s = StarState::new();
        s.target = ProjectionTarget { right_ascension: c.ra[0], declination: c.dec[0], angular_radius: 0.2 };
        s.project();
        let p = s.projected(0).expect("Sirius is in view");
        assert!(p.x.abs() < 1e-9 && p.y.abs() < 1e-9, "centred: {p:?}");

        // A star a little further east (larger RA) than the view centre lands on the left.
        let east = (0..c.len()).find(|&i| i != 0 && c.ra[i] > c.ra[0] && s.projected(i).is_some()).expect("a neighbour");
        assert!(s.projected(east).unwrap().x < 0.0, "east is left");
    }

    #[test]
    fn only_stars_within_the_radius_in_front_are_shown() {
        let mut s = StarState::new();
        s.target = ProjectionTarget { right_ascension: 0.0, declination: 0.0, angular_radius: 0.4 };
        s.project();
        assert!(!s.projection.is_empty());
        assert!(s.projection.iter().all(|p| p.x * p.x + p.y * p.y < 0.16));
        let wider = { let mut w = StarState::new(); w.target.angular_radius = 0.8; w.project(); w.projection.len() };
        assert!(wider > s.projection.len(), "a wider view shows more stars");
        assert!(s.projection.len() < 120, "a 0.4 rad view is a small patch: {}", s.projection.len());
    }

    #[test]
    fn wandering_sweeps_right_ascension_from_tau_to_zero_over_forty_seconds() {
        let mut s = StarState::new();
        s.tick(0.0);
        assert!((s.target.right_ascension - TAU).abs() < 1e-9 && s.target.declination == 0.0 && (s.target.angular_radius - 0.4).abs() < 1e-9);
        for _ in 0..200 { s.tick(0.1); } // 20 s
        assert!((s.target.right_ascension - TAU / 2.0).abs() < 1e-6, "half way round: {}", s.target.right_ascension);
        for _ in 0..200 { s.tick(0.1); } // 40 s: wraps
        assert!((s.target.right_ascension - TAU).abs() < 1e-6, "back at the start: {}", s.target.right_ascension);
    }

    #[test]
    fn flying_to_a_constellation_takes_ten_seconds_and_then_stays_put() {
        let mut s = StarState::new();
        s.tick(0.0);
        let o = orion();
        s.animate_to_constellation(o);
        s.mode = Mode::Constellation;
        s.tick(5.0);
        assert!(s.is_flying());
        s.tick(5.0);
        let c = &constellations()[o];
        assert!(!s.is_flying(), "done after 10 s");
        assert!((s.target.declination - c.center_dec).abs() < 1e-9);
        assert!((s.target.angular_radius - 1.2 * c.angular_radius).abs() < 1e-9);
        s.tick(30.0);
        assert!((s.target.right_ascension - c.center_ra).abs() < 1e-9, "pinned to the constellation");
    }

    #[test]
    fn the_fly_to_takes_the_short_way_round_in_right_ascension() {
        let mut s = StarState::new();
        s.target = ProjectionTarget { right_ascension: TAU - 0.1, declination: 0.0, angular_radius: 0.4 };
        let o = orion();
        s.animate_to_constellation(o);
        let c = &constellations()[o];
        // Orion is at about 1.4 rad; from 6.18 the short way is to go up through tau, so the
        // start is brought down by tau (to about -0.1) rather than sweeping 4.8 rad back.
        assert!(s.animate_from.right_ascension < c.center_ra + TAU / 2.0);
        assert!((s.animate_from.right_ascension - (TAU - 0.1 - TAU)).abs() < 1e-9);
    }

    #[test]
    fn stars_of_the_current_constellation_light_near_the_ball_and_only_once() {
        let mut s = StarState::new();
        let o = orion();
        s.animate_to_constellation(o);
        s.mode = Mode::Constellation;
        s.tick(10.0);
        let c = &constellations()[o];
        let star = *c.star_indices.iter().find(|&&i| s.projected(i).is_some()).expect("a star of Orion in view");
        let p = *s.projected(star).unwrap();
        let (nx, ny) = (p.x / s.target.angular_radius, p.y / s.target.angular_radius);

        assert_eq!(s.activate_near(nx + 0.5, ny + 0.5), 0, "too far away");
        assert!(s.activate_near(nx, ny) >= 1, "right on it");
        assert!(s.activated.contains(&star));
        assert_eq!(s.activate_near(nx, ny), 0, "lit once only");
        assert_eq!(s.activate_near(5.0, 5.0), 0, "outside the circle");
    }

    #[test]
    fn locking_and_resetting() {
        let mut s = StarState::new();
        s.animate_to_constellation(2);
        s.lock_current();
        s.lock_current();
        assert_eq!(s.locked, vec![2], "locked once");
        for i in 0..6 { s.animate_to_constellation(i); s.lock_current(); }
        assert!(s.all_constellations_locked());
        assert!(!s.switch_to_random_unlocked(|| 0.5), "nothing left to fly to");
        s.activated.insert(3);
        s.mode = Mode::Constellation;
        s.reset_and_wander();
        assert!(s.activated.is_empty() && s.locked.is_empty() && s.mode == Mode::Wandering && s.current.is_none());
    }

    #[test]
    fn the_random_pick_is_never_the_current_or_a_locked_constellation() {
        let mut s = StarState::new();
        s.current = Some(1);
        s.locked = vec![0, 2];
        for r in [0.0, 0.34, 0.67, 0.999] {
            let mut t = StarState::new();
            t.current = Some(1);
            t.locked = vec![0, 2];
            assert!(t.switch_to_random_unlocked(|| r));
            assert!(matches!(t.current, Some(3) | Some(4) | Some(5)), "picked {:?}", t.current);
            assert_eq!(t.mode, Mode::Constellation);
        }
        s.locked = vec![0, 2, 3, 4, 5];
        assert!(!s.switch_to_random_unlocked(|| 0.0), "only the current one is left");
    }
}
