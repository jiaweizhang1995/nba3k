use nba3k_core::*;

#[test]
fn cents_arithmetic() {
    let a = Cents::from_dollars(1_000_000);
    let b = Cents::from_dollars(500_000);
    assert_eq!((a + b).as_dollars(), 1_500_000);
    assert_eq!((a - b).as_dollars(), 500_000);
}

#[test]
fn ratings_overall_in_bounds() {
    let r = Ratings::legacy(99, 99, 99, 99, 99, 99, 99, 99);
    assert!(r.overall_estimate() <= 99);
    let zero = Ratings::default();
    assert_eq!(zero.overall_estimate(), 0);
}

#[test]
fn game_mode_parse() {
    assert_eq!(GameMode::parse("god"), Some(GameMode::God));
    assert_eq!(GameMode::parse("STANDARD"), Some(GameMode::Standard));
    assert_eq!(GameMode::parse("nope"), None);
    assert!(!GameMode::God.enforces_cba());
    assert!(GameMode::Standard.enforces_cba());
}

#[test]
fn gm_archetype_seeds_traits() {
    let p = GMPersonality::from_archetype("X", GMArchetype::Cheapskate);
    assert!(p.traits.tax_aversion > 1.5);
    let r = GMPersonality::from_archetype("Y", GMArchetype::Rebuilder);
    assert!(r.traits.potential_weight > r.traits.current_overall_weight);
}

#[test]
fn trade_offer_roundtrips_json() {
    use indexmap::IndexMap;
    let mut assets = IndexMap::new();
    assets.insert(TeamId(1), TradeAssets { players_out: vec![PlayerId(10)], picks_out: vec![], cash_out: Cents::ZERO });
    assets.insert(TeamId(2), TradeAssets { players_out: vec![PlayerId(20)], picks_out: vec![], cash_out: Cents::ZERO });
    let offer = TradeOffer { id: TradeId(1), initiator: TeamId(1), assets_by_team: assets, round: 1, parent: None };
    let s = serde_json::to_string(&offer).unwrap();
    let back: TradeOffer = serde_json::from_str(&s).unwrap();
    assert_eq!(back.assets_by_team.len(), 2);
    assert!(back.is_two_team());
}
