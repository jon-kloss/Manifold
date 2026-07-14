//! Transport route math (SDD §6, Addendum A3) — ONE params table drives rail,
//! truck, and drone; the Route Inspector renders whichever rows apply, the
//! empire recompute uses `throughput_per_min` as the route's capacity, and the
//! global solver picks transport kinds by the same thresholds. Throughput is
//! computed, never asserted (Principle 10).

use serde::{Deserialize, Serialize};

use crate::entities::{DroneSpec, RailSpec, TruckSpec};

/// Shared constants. v1 keeps speeds fixed and the rail headway penalty as the
/// one editable knob (A3.1: honest approximation over hidden precision —
/// signal-block modeling is v2).
pub struct TransportParams {
    pub rail_speed_kmh: f64,
    pub truck_speed_kmh: f64,
    pub drone_speed_kmh: f64,
    /// Planned path length × this = assumed track/road length (A3.1).
    pub terrain_factor: f64,
    /// Inventory slots: freight car 32, truck 48, drone 9.
    pub rail_slots_per_car: f64,
    pub truck_slots: f64,
    pub drone_slots: f64,
    /// Truck load+unload per end; drone takeoff+landing per trip.
    pub truck_load_s: f64,
    pub drone_ground_s: f64,
}

pub const PARAMS: TransportParams = TransportParams {
    rail_speed_kmh: 90.0,
    truck_speed_kmh: 40.0,
    drone_speed_kmh: 250.0,
    terrain_factor: 1.12,
    rail_slots_per_car: 32.0,
    truck_slots: 48.0,
    drone_slots: 9.0,
    truck_load_s: 25.0,
    drone_ground_s: 20.0,
};

/// Distance/rate thresholds for the wizard's transport pick (A3.3).
pub const BELT_MAX_M: f64 = 800.0;
pub const RAIL_MIN_RATE: f64 = 480.0;
pub const DRONE_MAX_RATE: f64 = 60.0;
pub const DRONE_MIN_M: f64 = 1500.0;

/// The math block (A3.1) — every line the inspector renders, in seconds and
/// items/min. `None` fields don't apply to the kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportMath {
    /// One-way path length × terrain factor, meters.
    pub effective_length_m: f64,
    pub round_trip_s: f64,
    pub load_unload_s: f64,
    /// Rail only: fixed editable headway penalty applied to travel time.
    pub headway_s: Option<f64>,
    pub rtt_s: f64,
    /// Items moved per full trip by one consist/truck/drone.
    pub per_trip_items: f64,
    /// All units combined (consists / trucks / the drone pair).
    pub throughput_per_min: f64,
    /// Drone only: batteries consumed per minute (solver-sourced later).
    pub batteries_per_min: Option<f64>,
    /// Truck only: the fuel item this route burns (line item in the block).
    pub fuel_item: Option<String>,
}

pub fn rail_math(path_len_m: f64, spec: &RailSpec, stack_size: f64) -> TransportMath {
    let len = path_len_m * PARAMS.terrain_factor;
    let travel_s = 2.0 * len / (PARAMS.rail_speed_kmh / 3.6);
    let dwell_s: f64 = spec.stations.iter().map(|s| s.dwell_s).sum();
    let headway_s = travel_s * spec.headway_penalty;
    let rtt_s = travel_s + dwell_s + headway_s;
    let per_trip = spec.cars as f64 * PARAMS.rail_slots_per_car * stack_size;
    let throughput = if rtt_s > 0.0 {
        spec.consists as f64 * per_trip / (rtt_s / 60.0)
    } else {
        0.0
    };
    TransportMath {
        effective_length_m: len,
        round_trip_s: travel_s,
        load_unload_s: dwell_s,
        headway_s: Some(headway_s),
        rtt_s,
        per_trip_items: per_trip,
        throughput_per_min: throughput,
        batteries_per_min: None,
        fuel_item: None,
    }
}

pub fn truck_math(path_len_m: f64, spec: &TruckSpec, stack_size: f64) -> TransportMath {
    let len = path_len_m * PARAMS.terrain_factor;
    let travel_s = 2.0 * len / (PARAMS.truck_speed_kmh / 3.6);
    let load_s = 2.0 * PARAMS.truck_load_s;
    let rtt_s = travel_s + load_s;
    let per_trip = PARAMS.truck_slots * stack_size;
    let throughput = if rtt_s > 0.0 {
        spec.trucks as f64 * per_trip / (rtt_s / 60.0)
    } else {
        0.0
    };
    TransportMath {
        effective_length_m: len,
        round_trip_s: travel_s,
        load_unload_s: load_s,
        headway_s: None,
        rtt_s,
        per_trip_items: per_trip,
        throughput_per_min: throughput,
        batteries_per_min: None,
        fuel_item: Some(spec.fuel_item.clone()),
    }
}

pub fn drone_math(path_len_m: f64, spec: &DroneSpec, stack_size: f64) -> TransportMath {
    // drones fly straight — no terrain factor (they're the one honest
    // exception; elevation-insensitive by design)
    let len = path_len_m;
    let travel_s = 2.0 * len / (PARAMS.drone_speed_kmh / 3.6);
    let ground_s = 2.0 * PARAMS.drone_ground_s;
    let rtt_s = travel_s + ground_s;
    let per_trip = PARAMS.drone_slots * stack_size;
    let throughput = if rtt_s > 0.0 {
        per_trip / (rtt_s / 60.0)
    } else {
        0.0
    };
    TransportMath {
        effective_length_m: len,
        round_trip_s: travel_s,
        load_unload_s: ground_s,
        headway_s: None,
        rtt_s,
        per_trip_items: per_trip,
        throughput_per_min: throughput,
        batteries_per_min: if rtt_s > 0.0 {
            Some(spec.batteries_per_trip / (rtt_s / 60.0))
        } else {
            None
        },
        fuel_item: None,
    }
}

/// The train answer-sheet (task #49): how many consists/trucks/drones a route
/// needs to serve a demand, computed from the SAME math block the inspector
/// renders. Pure and read-only — the renderer surfaces this for a PROSPECTIVE
/// route (before any rail is laid) as well as for an existing one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainAnswer {
    pub math: TransportMath,
    /// Throughput of ONE consist/truck/drone at these specs (items/min).
    pub per_train_per_min: f64,
    /// ceil(demand ÷ per-train) — the headline answer. 0 when demand is 0 or a
    /// single unit can move nothing.
    pub trains_needed: u32,
    pub demand_per_min: f64,
    /// Throughput at the CONFIGURED unit count − demand; negative ⇒ short.
    pub surplus_per_min: f64,
    /// The configured fleet can't meet demand (throughput < demand).
    pub short: bool,
}

/// ceil(demand ÷ per-train), guarding non-positive inputs (nothing to move, or
/// a unit that moves nothing → 0 trains).
pub fn trains_needed(demand_per_min: f64, per_train_per_min: f64) -> u32 {
    if demand_per_min <= 0.0 || per_train_per_min <= 0.0 {
        return 0;
    }
    (demand_per_min / per_train_per_min).ceil() as u32
}

/// Fold a computed math block, its unit count, and a demand into the answer.
/// `units` is the consist/truck count the math represents (drone = 1); dividing
/// it back out gives the per-train figure trains-needed ceils against.
pub fn train_answer(math: TransportMath, units: u32, demand_per_min: f64) -> TrainAnswer {
    let per_train = math.throughput_per_min / units.max(1) as f64;
    let surplus = math.throughput_per_min - demand_per_min;
    TrainAnswer {
        per_train_per_min: per_train,
        trains_needed: trains_needed(demand_per_min, per_train),
        demand_per_min,
        surplus_per_min: surplus,
        short: demand_per_min > math.throughput_per_min + 1e-6,
        math,
    }
}

/// Wizard transport pick (A3.3): belt under 800m; rail at distance or high
/// rate; drone for trickles over long hauls.
pub fn pick_transport(dist_m: f64, rate_per_min: f64) -> &'static str {
    if dist_m < BELT_MAX_M && rate_per_min < RAIL_MIN_RATE {
        "belt"
    } else if rate_per_min < DRONE_MAX_RATE && dist_m >= DRONE_MIN_M {
        "drone"
    } else {
        "rail"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::StationSpec;

    fn spec() -> RailSpec {
        RailSpec {
            consists: 1,
            locos: 2,
            cars: 6,
            stations: vec![
                StationSpec {
                    name: "A".into(),
                    platforms: 1,
                    dwell_s: 25.0,
                },
                StationSpec {
                    name: "B".into(),
                    platforms: 1,
                    dwell_s: 25.0,
                },
            ],
            headway_penalty: 0.15,
        }
    }

    #[test]
    fn rail_block_matches_the_a3_example_shape() {
        // A3.1: 2×3.4km @ ~90km/h ≈ 4:32 travel, 0:50 dwell, 15% headway
        let m = rail_math(3400.0 / PARAMS.terrain_factor, &spec(), 100.0);
        assert!(
            (m.round_trip_s - 272.0).abs() < 1.0,
            "travel {}",
            m.round_trip_s
        );
        assert!((m.load_unload_s - 50.0).abs() < 1e-9);
        assert!((m.headway_s.unwrap() - 272.0 * 0.15).abs() < 1.0);
        assert!((m.rtt_s - (272.0 + 50.0 + 40.8)).abs() < 1.5);
        // throughput = 6 cars × 32 slots × stack / rtt_min, scaled by consists
        let expected = 6.0 * 32.0 * 100.0 / (m.rtt_s / 60.0);
        assert!((m.throughput_per_min - expected).abs() < 1e-6);
        // one more consist doubles it
        let mut s2 = spec();
        s2.consists = 2;
        let m2 = rail_math(3400.0 / PARAMS.terrain_factor, &s2, 100.0);
        assert!((m2.throughput_per_min - 2.0 * expected).abs() < 1e-6);
    }

    #[test]
    fn drone_burns_batteries_per_trip() {
        let m = drone_math(
            2000.0,
            &DroneSpec {
                batteries_per_trip: 4.0,
            },
            50.0,
        );
        assert!(m.batteries_per_min.unwrap() > 0.0);
        assert!(m.throughput_per_min > 0.0);
        // straight-line: no terrain factor
        assert!((m.effective_length_m - 2000.0).abs() < 1e-9);
    }

    #[test]
    fn trains_needed_is_ceil_division() {
        // 1000/min demand, a consist that moves 300/min → 4 trains (ceil 3.33).
        assert_eq!(trains_needed(1000.0, 300.0), 4);
        // an exact multiple never rounds up.
        assert_eq!(trains_needed(900.0, 300.0), 3);
        // no demand, or a unit that can't move anything → zero trains.
        assert_eq!(trains_needed(0.0, 300.0), 0);
        assert_eq!(trains_needed(500.0, 0.0), 0);
    }

    #[test]
    fn train_answer_classifies_short_and_surplus() {
        let m = rail_math(3400.0 / PARAMS.terrain_factor, &spec(), 100.0);
        let per_train = m.throughput_per_min; // spec() is a single consist
                                              // demand just over one train → SHORT at the configured 1 consist, needs 2.
        let a = train_answer(m.clone(), 1, per_train + 10.0);
        assert!(a.short);
        assert!(a.surplus_per_min < 0.0);
        assert_eq!(a.trains_needed, 2);
        assert!((a.per_train_per_min - per_train).abs() < 1e-6);
        // demand under one train → surplus, one train suffices.
        let b = train_answer(m, 1, per_train - 10.0);
        assert!(!b.short);
        assert!(b.surplus_per_min > 0.0);
        assert_eq!(b.trains_needed, 1);
    }

    #[test]
    fn thresholds_pick_sane_kinds() {
        assert_eq!(pick_transport(400.0, 60.0), "belt");
        assert_eq!(pick_transport(1200.0, 200.0), "rail");
        assert_eq!(pick_transport(500.0, 700.0), "rail"); // rate forces rail
        assert_eq!(pick_transport(2000.0, 30.0), "drone");
    }
}
