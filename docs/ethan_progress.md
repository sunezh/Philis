# Philis — Verify Module

## Learning (will update)

### Background reading
  * Razavi chapters 2.4 & 19
  * Notes: <https://uofwaterloo-my.sharepoint.com/:w:/g/personal/e58sun_uwaterloo_ca/IQDm08PNRQkoQ66RDNlMwwZHAd2ZxvyAAriIXxyglE9QQII?e=1eTNze>
* Understanding Verify Module: <https://uw-asic.github.io/Philis/Developer/verify/index.html>

  
### Programming resources used

  [rust-guide-for-philis.html 67270](/api/attachments.redirect?id=9c4716ed-ecc0-4476-bcab-4b9bde3769f2)

  [python_guide_ethan.html 48813](/api/attachments.redirect?id=f623dbe2-99f3-4918-91a1-6ba148b342b4)



## Verify Module Progress

### Completed


1. Minimal implementation of Rect
2. DRC types and basic predicates
   * DRC: ViolationKind/DrcViolation/ShapeRef/RuleId/LayerId
   * Basic Predicates: check_spacing/check_widtth/check_area/check_overlap/check_grid_snap/check_outline_exceed
3. DRC advanced predicates
    * check_enclosure, check_eol_spacing, check_prl_spacing, check_cut_spacing, check_same_net_notch, check_well_spacing, check_well_enclosure, check_implant_spacing + orthogonality tests
4. DRC incremental infra
    *  DrcRegion, DrcDelta, RuleFamilyCoverage; compute_r_max deferred (needs philis-tech::RuleSet, which doesn't exist yet)

Note: Demo for work up to this point: cargo run -p philis-verify --example drc_demo


### In Progress


 5. LVS union-find
 6. LVS connectivity
 7. PEX wire models
 8. PEX coupling models
 9. PEX aggregation
10. EM checking
11. IR drop
12. Antenna checking

Phase 2 — DRC advanced predicates: check_enclosure, check_eol_spacing, check_prl_spacing, check_cut_spacing, check_same_net_notch, check_well_spacing, check_well_enclosure, check_implant_spacing + orthogonality tests
Phase 3 — DRC incremental infra: DrcRegion, DrcDelta, RuleFamilyCoverage; compute_r_max deferred (needs philis-tech::RuleSet, which doesn't exist yet)
Phase 4 — LVS union-find (ConnectivityTracker, NetId, UnionResult, LvsResult/LvsOpen/LvsShort/LvsMismatch) — self-contained
Phase 5 — LVS connectivity extraction (extract_connectivity)
Phase 6 — PEX wire models (wire_resistance, ground_capacitance, via_resistance)
Phase 7 — PEX coupling models (same_layer_coupling, interlayer_coupling)
Phase 8 — PEX aggregation (NetParasitics, SegmentParasitics, detect_outliers, PexOutlier)
Phase 9 — EM checking (current_density, check_em, EmViolation)
Phase 10 — IR drop (compute_ir_drop, IrDropResult — resistive mesh solve, highest complexity)
Phase 11 — Antenna checking (check_antenna, AntennaViolation)