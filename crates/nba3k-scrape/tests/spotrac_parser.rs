use nba3k_scrape::sources::spotrac::parse_future_picks_html;

#[test]
fn parses_future_pick_rows_and_protection_text() {
    let html = r#"
      <div class="tab-pane" id="round1_100">
        <table><tbody>
          <tr><td><h2>2026</h2></td></tr>
          <tr>
            <td class="center"><img src="https://media.spotrac.com/images/thumb/nba_det.png" /></td>
            <td class="center" colspan="30">
              <div style="font-weight: bold;">DET</div>
            </td>
          </tr>
          <tr>
            <td class="center"><img src="https://media.spotrac.com/images/thumb/nba_min.png" /></td>
            <td class="center" colspan="30">
              <div style="font-weight: bold;"><i class="fa-solid fa-arrow-right-long"></i> DET</div>
              <div style="font-size:8px">DET can swap for MIN</div>
            </td>
          </tr>
          <tr><td><h2>2027</h2></td></tr>
          <tr>
            <td class="center"><img src="https://media.spotrac.com/images/thumb/nba_bkn.png" /></td>
            <td class="center" colspan="30">
              <div style="font-weight: bold;"><i class="fa fa-refresh white"></i> HOU</div>
              <div style="font-size:8px">top-4 protected</div>
            </td>
          </tr>
        </tbody></table>
      </div>
      <div class="tab-pane" id="round2_100">
        <table><tbody>
          <tr><td><h2>2026</h2></td></tr>
          <tr>
            <td class="center"><img src="https://media.spotrac.com/images/thumb/nba_gs.png" /></td>
            <td class="center" colspan="30"><div style="font-weight: bold;">GSW</div></td>
          </tr>
        </tbody></table>
      </div>
    "#;

    let rows = parse_future_picks_html(html).expect("parse");
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().any(|p| {
        p.year == 2027
            && p.round == 1
            && p.original_team_abbrev == "BRK"
            && p.current_owner_abbrev == "HOU"
            && p.is_swap
            && p.protection_text.as_deref() == Some("top-4 protected")
    }));
    assert!(rows.iter().any(|p| {
        p.year == 2026
            && p.round == 2
            && p.original_team_abbrev == "GSW"
            && p.current_owner_abbrev == "GSW"
    }));
}
