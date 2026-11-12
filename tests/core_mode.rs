use bosun::core::approve::Policy;
use bosun::core::mode::*;

#[test]
fn ladder_order_and_names() {
    assert!(ModeName::Gatekeeper < ModeName::Steady);
    assert!(ModeName::Steady < ModeName::Booster);
    assert!(ModeName::Booster < ModeName::Autonomous);
    assert_eq!(ModeName::Booster.as_str(), "booster");
}

#[test]
fn presets_follow_the_table() {
    let g = preset(ModeName::Gatekeeper);
    assert_eq!(
        (g.push_gates, g.heal_pr, g.heal_main, g.approvals_batch),
        (
            PushGates::Ready,
            HealPr::Off,
            HealMain::Off,
            ApprovalsBatch::PerPr
        )
    );
    assert_eq!(
        (g.spawn_auto, g.budget_feature, g.brake, g.curator_loosen),
        (
            SpawnAuto::Off,
            BudgetFeature::Off,
            Brake::Off,
            CuratorLoosen::Off
        )
    );
    let s = preset(ModeName::Steady);
    assert_eq!(s.push_gates, PushGates::Push);
    assert_eq!(s.heal_pr, HealPr::Notify);
    assert_eq!(s.heal_main, HealMain::RevertPr);
    assert_eq!(s.approvals_batch, ApprovalsBatch::Daily);
    assert_eq!(s.spawn_auto, SpawnAuto::Off);
    assert_eq!(s.budget_feature, BudgetFeature::Warn);
    assert_eq!((s.brake, s.curator_loosen), (Brake::On, CuratorLoosen::Off));
    let b = preset(ModeName::Booster);
    assert_eq!((b.heal_pr, b.spawn_auto), (HealPr::Spawn, SpawnAuto::On));
    assert_eq!(b.budget_feature, BudgetFeature::Ask);
    assert_eq!(b.curator_loosen, CuratorLoosen::Propose);
    assert_eq!(preset(ModeName::Autonomous).heal_main, HealMain::RevertAuto);
}

#[test]
fn preset_policy_only_for_the_fast_lane() {
    assert_eq!(preset_policy(ModeName::Gatekeeper), None);
    assert_eq!(preset_policy(ModeName::Steady), None);
    assert_eq!(preset_policy(ModeName::Booster), Some(Policy::Autonomous));
    assert_eq!(
        preset_policy(ModeName::Autonomous),
        Some(Policy::Autonomous)
    );
}

#[test]
fn resolve_sources() {
    let e = resolve(&ModeConfig::default());
    assert_eq!(e.mode, ModeName::Gatekeeper);
    assert_eq!(e.mode_source, Source::Default);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::Off,
            source: Source::Default
        }
    );
    let e = resolve(&ModeConfig {
        name: Some(ModeName::Booster),
        ..ModeConfig::default()
    });
    assert_eq!(e.mode_source, Source::Explicit);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::RevertPr,
            source: Source::Preset
        }
    );
    let mut o = ModeConfig {
        name: Some(ModeName::Booster),
        heal_main: Some(HealMain::Off),
        ..ModeConfig::default()
    };
    let e = resolve(&o);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::Off,
            source: Source::Explicit
        }
    );
    assert_eq!(
        e.knobs.spawn_auto,
        Set {
            value: SpawnAuto::On,
            source: Source::Preset
        }
    );
    o.name = None;
    let e = resolve(&o);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::Off,
            source: Source::Explicit
        }
    );
    assert_eq!(
        e.knobs.brake,
        Set {
            value: Brake::Off,
            source: Source::Default
        }
    );
}

fn win(main_red: u32, reverts: u32, failing: u32, pushed: u32) -> BrakeWindow {
    BrakeWindow {
        days: 7,
        main_red,
        reverts,
        failing_heads: failing,
        pushed_heads: pushed,
    }
}

#[test]
fn brake_rules() {
    let m = ModeName::Steady;
    assert!(!brake(m, &win(1, 1, 0, 10)).on);
    let b = brake(m, &win(2, 0, 0, 10));
    assert!(b.on && b.reasons.len() == 1, "{b:?}");
    assert!(brake(m, &win(0, 2, 0, 10)).on);
    // steady target 30: 39 % is quiet, 40 % trips
    assert!(!brake(m, &win(0, 0, 39, 100)).on);
    assert!(brake(m, &win(0, 0, 40, 100)).on);
    // booster target 20, autonomous 15
    assert!(brake(ModeName::Booster, &win(0, 0, 30, 100)).on);
    assert!(!brake(ModeName::Booster, &win(0, 0, 29, 100)).on);
    assert!(brake(ModeName::Autonomous, &win(0, 0, 25, 100)).on);
}

#[test]
fn brake_edge_cases() {
    assert!(!brake(ModeName::Steady, &win(0, 0, 0, 0)).on);
    assert!(!brake(ModeName::Steady, &win(0, 0, 5, 0)).on);
    // gatekeeper has no rate target
    assert!(!brake(ModeName::Gatekeeper, &win(0, 0, 100, 100)).on);
    assert!(brake(ModeName::Gatekeeper, &win(2, 0, 0, 0)).on);
    let b = brake(ModeName::Steady, &win(2, 2, 100, 100));
    assert_eq!(b.reasons.len(), 3);
}
