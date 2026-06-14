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
   * DRC: ViolationKind, DrcViolation, ShapeRef, RuleId, LayerId
   * Basic Predicates: check_spacing, check_widtth, check_area, check_overlap, check_grid_snap, check_outline_exceed
3. DRC advanced predicates
    * check_enclosure, check_eol_spacing, check_prl_spacing, check_cut_spacing, check_same_net_notch, check_well_spacing, check_well_enclosure, check_implant_spacing + orthogonality tests
4. DRC incremental infra
    *  DrcRegion, DrcDelta, RuleFamilyCoverage; compute_r_max deferred (needs philis-tech::RuleSet, which doesn't exist yet)

Demo for DRC: cargo run -p philis-verify --example drc_demo

5. LVS union-find
    * ConnectivityTracker, NetId, UnionResult, LvsResult, LvsOpen, LvsShort, LvsMismatch
6. LVS connectivity

Demo for LVS: cargo run -p philis-verify --example lvs_demo

7. PEX wire models
     * wire_resistance, ground_capacitance, via_resistance
8. PEX coupling models
    * same_layer_coupling, interlayer_coupling
9. PEX aggregation
    * NetParasitics, SegmentParasitics, detect_outliers, PexOutlier

Demo for PEX: cargo run -p philis-verify --example pex_demo

10. EM checking
    * current_density, check_em, EmViolation
11. IR drop
    * compute_ir_drop, IrDropResult — resistive mesh solve, highest complexity
12. Antenna checking
    * check_antenna, AntennaViolation

Demo for EM/IR/Antenna: 

Final Demo: cargo run -p philis-verify --example verify_demo