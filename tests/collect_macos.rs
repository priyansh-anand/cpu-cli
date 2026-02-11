mod common;

use cpu_cli::units::Bytes;

#[test]
fn apple_m5_has_named_clusters_and_no_diagnostics() {
    let cpu = common::load_named("apple-m5");
    assert_eq!(cpu.identity.name.as_ref().unwrap().value, "Apple M5");
    let labels: Vec<&str> = cpu.clusters.iter().map(|c| c.label()).collect();
    assert_eq!(labels, ["Super", "Efficiency"]);
    assert!(cpu.diagnostics.is_empty(), "{:?}", cpu.diagnostics);
}

#[test]
fn apple_m2_pro_has_two_performance_l2_instances() {
    let cpu = common::load_named("apple-m2-pro");
    let l2 = cpu.clusters[0]
        .caches
        .iter()
        .find(|k| k.level == 2)
        .unwrap();
    assert_eq!(l2.size.as_ref().unwrap().value, Bytes(16 << 20));
    assert_eq!(l2.instances.as_ref().unwrap().value, 2);
}

#[test]
fn sparse_mac_is_identified_without_guessed_caches() {
    let cpu = common::load_named("sparse-mac");
    assert!(cpu.is_identified());
    assert_eq!(cpu.clusters.len(), 1);
    assert!(cpu.clusters[0].caches.is_empty());
    assert!(cpu.features.is_empty());
}
