//! Round-trip every `DaemonErrorWire` variant through serde.
//!
//! `DaemonErrorWire` is the wire-format enum sent across the daemon socket.
//! Forgetting a `serde` derive on a new variant only manifests at runtime
//! today; this suite catches it at test time and (via
//! `all_variants_for_test`'s exhaustive sentinel match) catches missing
//! coverage at compile time.

use swell_daemon::error::DaemonErrorWire;

#[test]
fn every_daemon_error_wire_round_trips_through_serde() {
    for variant in DaemonErrorWire::all_variants_for_test() {
        let json =
            serde_json::to_string(&variant).expect("DaemonErrorWire must be serializable");
        let back: DaemonErrorWire = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("failed to deserialize {variant:?}: {e}"));

        assert_eq!(
            format!("{variant:?}"),
            format!("{back:?}"),
            "round-trip mismatch for variant: {json}",
        );
    }
}
