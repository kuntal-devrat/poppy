use poppy_core::TimeBase;
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_timebase_rescale_roundtrip(ts in -1_000_000_000i64..1_000_000_000i64) {
        let a = TimeBase::new(1, 90000);
        let b = TimeBase::new(1, 1000);
        let mid = a.rescale(ts, b);
        let back = b.rescale(mid, a);
        // Roundtrip within 1 tick (integer rounding).
        prop_assert!((back - ts).abs() <= 100);
    }

    #[test]
    fn prop_timebase_compare_consistent(a in -1_000_000i64..1_000_000i64, b in -1_000_000i64..1_000_000i64) {
        let ta = TimeBase::new(1, 90000);
        let tb = TimeBase::new(1, 48000);
        let ord = ta.compare(a, tb, b);
        // Cross-check with i128 exact math.
        let lhs = a as i128 * 48000;
        let rhs = b as i128 * 90000;
        prop_assert_eq!(ord, lhs.cmp(&rhs));
    }

    #[test]
    fn prop_seek_ticks_finite(sec in -1000.0f64..10000.0f64) {
        let tb = TimeBase::new(1, 90000);
        let ticks = tb.from_seconds(sec);
        let sec_back = tb.to_seconds(ticks);
        // Roundtrip within one tick period plus epsilon for float precision
        prop_assert!((sec_back - sec).abs() <= 1.0 / 90000.0 + 1e-6);
        let _ = tb.to_micros(ticks);
    }

    #[test]
    fn prop_from_seconds_non_finite_maps_to_zero(flag in 0u8..3u8) {
        let tb = TimeBase::new(1, 90000);
        let val = match flag {
            0 => f64::NAN,
            1 => f64::INFINITY,
            _ => f64::NEG_INFINITY,
        };
        prop_assert_eq!(tb.from_seconds(val), 0);
    }
}
