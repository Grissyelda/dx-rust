//! The Nameless Mod, Mission 25: the intro, the training course, the endings and the epilogue. Written for DX RUST
//! from how these maps behave in the player's own copy of the mod; tags, flags, map and conversation names are the
//! identifiers its maps use.
//!
//! Covered: each ending map's cast (who appears and where, by the choices made), the forced conversations that
//! carry the cut scenes, the training course's tutorial messages and skill boosts.
//!
//! Also: the ending quotes, the mod's own credits roll and the slow-motion scene.
//!
//! Also: the fades (fade.rs), the ban effect (tnmban.rs) and the achievements (tnmach.rs).

use super::*;

/// One ending's quote (the strings and the time come from the mission script's defaults in the player's copy).
fn quote(n: usize) {
    let lines: Vec<String> = (0..2).map(|i| text(&format!("endgameQuote[{}]", 2 * n + i))).collect();
    let secs = text(&format!("endgameDelays[{n}]")).parse::<f32>().unwrap_or(10.0);
    crate::classic_hud::quote_text(&lines, secs);
}

/// Trestkon's stand-ins in the cut scenes: (navigation point, tag, BindName or "").
fn spawn_trestkon(c: &Ctx, at: &str, tag: &str, bind: &str, orders: (&str, &str)) {
    let Some((pos, yaw)) = c.point("PathNode", at).or_else(|| c.point("PatrolPoint", at)).or_else(|| {
        c.lvl.actors.iter().find(|a| tag_of(&a.props).eq_ignore_ascii_case(at)).map(|a| {
            let l = a.location();
            (DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), 0)
        })
    }) else {
        return;
    };
    let mut props = vec![
        prop("Tag", Value::Name(tag.into())),
        prop("Orders", Value::Name(orders.0.into())),
        prop("GroundSpeed", Value::Float(180.0)),
    ];
    if !orders.1.is_empty() {
        props.push(prop("OrderTag", Value::Name(orders.1.into())));
    }
    if !bind.is_empty() {
        props.push(prop("BindName", Value::Str(bind.into())));
    }
    if let Ok(mut q) = PAWNS.lock() {
        q.push(("Treskton".into(), pos, yaw, props));
    }
    if !bind.is_empty() {
        crate::consys::bind_tag(tag, bind, "");
    }
}

pub(super) fn first(c: &mut Ctx, map: &str) {
    // uc: tnmMission25.FirstFrame
    // Reaching an ending unlocks the invasions (Trestkon.bUnlockedSword, saved with the player's settings).
    if map.starts_with("TNMENDGAME") {
        crate::tnminvasion::unlock();
    }
    for a in ["AugLight", "AugCombat", "AugMuscle", "AugTarget", "AugVision", "AugCloak", "AugRadarTrans"] {
        crate::abilities::deactivate_aug(a);
    }
    if map.to_ascii_uppercase().starts_with("TNMENDGAME") && crate::carry::carrying() {
        crate::carry::discard_body();
    }
    match map {
        "25_TNMTRAINING" => {
            if let Some(a) = crate::abilities::lock().as_mut() {
                a.skills.points_avail = 0;
                a.skills.points_total = 0;
            }
            c.trigger("tnmCameraPoint");
        }
        "TNMINTRO" => {
            if !c.b("MS_Intro_Played") {
                c.trigger("Seq01");
                c.setd("MS_Intro_Played", true);
            }
        }
        "TNMENDGAME01" => {
            c.trigger("ScaraCam");
            spawn_trestkon(c, "TrestkonSpawn1", "TrestkonSS", "TrestkonSS", ("Standing", ""));
            spawn_trestkon(c, "TrestkonSpawn2", "Trestkon", "", ("Standing", ""));
            if c.b("TRM_Dead") || !c.b("TRM_Done") {
                c.enter_tag("DXOGuysMeeting");
            } else {
                c.enter_tag("DXOGuysLeaving");
                c.enter_tag("Compudoc");
            }
        }
        "TNMENDGAME02" => {
            c.trigger("PDXHQCam01");
            if c.b("Scara_Banned") {
                c.leave_tag("ScaraKing");
                spawn_trestkon(c, "WC03", "treskton", "", ("Standing", ""));
                c.enter_tag("ThatGuy");
            } else {
                spawn_trestkon(c, "TrestkonSpawn", "treskton", "", ("Standing", ""));
                c.enter_tag("ThatGuy_Prop");
            }
            if c.any(&["Kylie_Dislike", "Kylie_Unconscious", "Kylie_Dead"]) {
                c.setd("Kylie_NotPresent", true);
            }
            if c.b("Ryan_Banned") {
                c.leave_tag("Ryan");
            }
        }
        "TNMENDGAME03" => {
            if c.b("ENDING_Narc") {
                spawn_trestkon(c, "NarcTrestkonPoint01", "TrestkonNarc", "", ("Standing", ""));
                c.trigger("Narc01Cam");
                c.vars.endgame_timer = 1.0;
            } else {
                c.trigger("SSBlowup1");
            }
            spawn_trestkon(c, "TrestkonSpawn", "TrestkonRyan", "", ("Standing", ""));
            spawn_trestkon(c, "NarcTrestkonPointDT", "CircleTrestkon", "TresktonDT", ("Standing", ""));
            c.leave_tag(if c.b("ALLIED_WC") { "Kevo" } else { "Midashand" });
        }
        "TNMDENOUEMENT" => {
            c.trigger("Cam00");
            if c.b("ALLIED_WC") && c.b("AlphaOperator_Dead") {
                c.leave_tag("SolsPatron");
            }
            if c.b("Ryan_Banned") {
                c.leave_tag("Ryan");
            }
            if c.b("ALLIED_WC") && c.any(&["Yardbomb_AdvisedToSell", "Yardbomb_Dead"]) {
                if !c.b("Yardbomb_Dead") {
                    c.leave_tag("Yardbomb");
                }
                c.enter_tag("VoodooWCGuys");
            }
            if ["Athena", "Beefman", "DarkTemplar", "Abomination"].iter().all(|n| c.b(&format!("{n}_Dead"))) {
                c.leave_tag("Goats");
                c.leave_tag("OiNutter");
            }
            if c.b("ALLIED_WC") && c.b("Kylie_Dislike") && !c.b("Kylie_Dead") {
                c.enter_tag("Kylie_ZP");
            }
            spawn_trestkon(c, "TrestkonSpawn_Partyzone", "TrestkonPZ", "", ("Standing", ""));
            spawn_trestkon(c, "TrestkonSpawn_Scara", "TrestkonWC", "", ("Sitting", "ScaraChair"));
            spawn_trestkon(c, "TrestkonSpawn_Final", "Trestkon", "", ("Standing", ""));
        }
        _ => {}
    }
}

pub(super) fn timer(c: &mut Ctx, map: &str) {
    match map {
        "25_TNMTRAINING" => {
            c.set("PM_Phas_DrinkWine_Played", false, 0);
            if c.count("TNMPawn", Some("CameraDummy")) == 0 {
                c.pm("PM_Phas_Tut10");
            }
            if c.count("TNMPawn", Some("FistDummy")) == 0 {
                c.pm("PM_Phas_Tut12");
            }
            if c.inv_classes.iter().any(|k| k.eq_ignore_ascii_case("WeaponMolBottle")) {
                c.pm("PM_Phas_Tut15");
            }
            if crate::abilities::aug_level("AugLight") == Some(1) {
                c.pm("PM_Phas_Tut19");
            }
            if crate::abilities::aug_level("AugCombat") == Some(1) || crate::abilities::aug_level("AugMuscle") == Some(1) {
                c.pm("PM_Phas_Tut20");
            }
        }
        // The Narcissus ending opens with its first conversation (the map's own start, a tick after arrival).
        "TNMENDGAME03" if c.vars.endgame_timer > 0.0 => {
            if play(c, "NarcEndgame01") {
                c.vars.endgame_timer = 0.0;
            }
        }
        _ => {}
    }
}

fn play(c: &mut Ctx, con: &str) -> bool {
    println!("MISSION forced conversation {con}");
    crate::consys::start_forced_con(c.fl, con)
}

fn set_skill(class: &str, level: usize) {
    if let Some(a) = crate::abilities::lock().as_mut() {
        a.skills.set_level(class, level);
    }
}

pub(super) fn event(c: &mut Ctx, f: &str) -> bool {
    let patrol = |c: &Ctx, tag: &str, point: &str| c.orders(Sel::tag(tag), "Patrolling", point);
    match f {
        "ActivateCamera" => {
            crate::security::event("*noalarm:TrainingCamera", false);
            crate::security::event("TrainingCamera", true);
        }
        "MaxComputers" => set_skill("SkillComputer", 3),
        "UpgradeDemolitions" => set_skill("tnmSkillDemolition", 1),
        "MaxDemolitions" => set_skill("tnmSkillDemolition", 3),
        "MaxFists" => set_skill("SkillWeaponFists", 3),
        "GiveTargettingAug" => {
            if let Some(a) = crate::abilities::lock().as_mut() {
                a.augs.give("AugTarget", false);
                for _ in 0..4 {
                    a.augs.give("AugTarget", true);
                }
            }
        }
        "HidePhas" => c.leave_tag("HoloPhas"),
        "UnHidePhas" => c.enter_tag("HoloPhas"),
        "RemoveAllInventory" => player_req(PlayerReq::RemoveAll),
        "SendMoreWine" => {
            c.spawn_item("WineBottleTrigger", "SpawnMoreWine");
            for p in c.points_named("SpawnMoreWine") {
                crate::dynlight::explosion(p, 8.0, 128, 96);
                c.fx(Fx::Sphere(p, 4.0));
            }
        }
        // BanPawn / BanScaraFX: BanInfo.StartBan's fade on each of them (tnmban.rs).
        "BanFighters" => {
            crate::tnmban::start_tag("LlamaFighter");
            crate::tnmban::start_tag("GoatFighter");
        }
        "BanWCThugs" => crate::tnmban::start_tag("SlumFighter3"),
        "BanScara" => crate::tnmban::start_tag("ScaraKing"),
        "DDAppear" => {
            c.enter_tag("DD");
            patrol(c, "DD", "DDWaypoint01");
        }
        "DDEquipSword" => {}
        "DDEnterAlley" => patrol(c, "DD", "DDAlleyWaypoint"),
        "DDVanish" => c.leave_tag("DD"),
        "DDSlumsAppear" => {
            c.enter_tag("dd");
            patrol(c, "dd", "DDPosition");
        }
        "PlayIntro1" | "PlayIntro2" | "PlayIntro3" | "PlayIntro4" | "PlayIntro5" | "PlayIntro6" => {
            let n = &f["Play".len()..];
            play(c, n);
        }
        "ArmThugs" => {}
        "ClearCheckpoint" => {
            for t in ["CheckpointBusinessman", "CheckpointThugs", "CheckpointBystanders"] {
                c.leave_tag(t);
            }
            c.vehicle(ActorSel::tag("CheckpointCars"), false);
        }
        "SpawnCultWar" => {
            c.enter_tag("Llamas");
            c.enter_tag("Goats");
            c.show("CultCorpses", true);
        }
        "KashueLeave" => {
            c.enter_tag("KashueLeaving");
            patrol(c, "KashueLeaving", "KKWaypoint");
        }
        "TrestkonAppear" => spawn_trestkon(c, "TrestkonSpawn", "Trestkon", "", ("Patrolling", "TrestkonWaypoint")),
        // The intro is over: the new game it was started for, else back to the title (Trestkon.PostIntro).
        "EndIntro" => crate::cinematic::post_intro(),
        // PrintEndgameQuote: two lines of the script's endgameQuote list, centred, for endgameDelays[n] seconds.
        "PrintEndgameQuoteABI" => quote(0),
        "PrintEndgameQuoteWC1" => quote(1),
        "PrintEndgameQuoteWC2" => quote(2),
        "PrintEndgameQuotePDX" => quote(3),
        "PrintEndgameQuoteNARC" => quote(4),
        // RollCredits: the mod's credits window (its banner, TNMText.TNMCredits, its Ogg track), then the title.
        "RollCredits" => crate::credits::show(),
        "PlayPDXEndgame01" => {
            play(c, "PDXEndgame01");
            // ScaraKing goes to state ScaraBan: LoopAnim('Shocked'), ignoring everything.
            crate::ai::world_req(crate::ai::WorldReq::LoopAnim(crate::ai::Sel::tag("ScaraKing"), "Shocked"));
            crate::tnmach::unlock("achEndingPDX");
        }
        "PlayPDXEndgame02" => {
            play(c, "PDXEndgame02");
        }
        "PlayPDXEndgame03" => {
            play(c, "PDXEndgame03");
        }
        "TrestkonMoveUp" => patrol(c, "Trestkon", "TrestkonDestination"),
        "TRMAppear" => {
            c.enter_tag("TRM");
            patrol(c, "TRM", "TRMPosition");
        }
        "DXOLightsOut" => c.trigger("DXOLight"),
        "PlayWCEndgame01" => {
            play(c, "WCEndgame01");
            crate::tnmach::unlock("achEndingWC");
        }
        "PlayWCEndgame02" => {
            let con = if c.b("Scara_Banned") { "WCEndgame04" } else { "WCEndgame02" };
            play(c, con);
        }
        "KylieAppear" => {
            c.enter_tag("Kylie");
            let p = if c.b("Scara_Banned") { "WC02" } else { "WC01" };
            patrol(c, "Kylie", p);
        }
        "PlayWCEndgameKylie" => {
            let con = if c.b("Scara_Banned") { "WCEndgame05" } else { "WCEndgame03" };
            play(c, con);
        }
        "ScaraLeave" => {
            for t in ["ScaraKing", "ZeroPresence", "ThatGuy_Prop"] {
                c.orders(Sel::tag(t), "Leaving", "WCLeavePoint");
            }
        }
        "ThatGuyAndZPLeave" => {
            for t in ["ThatGuy", "ZeroPresence"] {
                c.orders(Sel::tag(t), "Leaving", "WCLeavePoint");
            }
        }
        "ShuttleSeq02Appear" => c.vehicle(ActorSel::tag("ShuttleSeq02"), true),
        "ShuttleReturnAppear" => {
            c.vehicle(ActorSel::tag("ShuttleReturn"), true);
            c.show("SpaceAlienCarc", true);
            if !c.b("Kylie_Dead_M24") && !c.b("Kylie_Deleted") {
                c.show("KylieCarcass", true);
            }
        }
        "TrestkonLeaveShuttle" => patrol(c, "TrestkonRyan", "TrestkonRyanPosition"),
        "PlayABIEndgame01" => {
            play(c, "ABIEndgame01");
            crate::tnmach::unlock("achEndingABI");
        }
        "PlayABIEndgame02" => {
            play(c, "ABIEndgame02");
        }
        "TrestkonLeaveABI" => patrol(c, "TrestkonRyan", "TrestkonLeavePoint"),
        "TrestWalkToPoint02" | "TrestWalkToPoint03" | "TrestWalkToPoint04" => {
            let n = &f[f.len() - 2..];
            patrol(c, "TrestkonNarc", &format!("NarcTrestkonPoint{n}"));
        }
        "PlayNarcEndgame02" | "PlayNarcEndgame03" | "PlayNarcEndgame04" | "PlayNarcEndgame05" => {
            let n = &f["Play".len()..];
            play(c, n);
            if n == "NarcEndgame02" {
                crate::tnmach::unlock("achEndingNarc");
            }
        }
        "FadeInAliens" => {
            c.enter(Sel::isa("Alien"));
            crate::fade::request(crate::fade::Who::Class("Alien".into()), 5.0, true);
        }
        "FadeOutTrestkon" => crate::fade::request(crate::fade::Who::Tag("CircleTrestkon".into()), 2.0, false),
        "SlowDown1" => crate::tnmmode::set_speed(0.5),
        "SlowDown2" => crate::tnmmode::set_speed(0.25),
        "NormalSpeed" => crate::tnmmode::set_speed(1.0),
        // NarcFadeInInfo: the CirclePawns enter and fade in one after another, two seconds each.
        "FadeInCircle" => {
            let circle: Vec<usize> = c
                .lvl
                .actors
                .iter()
                .enumerate()
                .filter(|(_, a)| crate::classtree::is_a(&a.class, "TNMPawn") && tag_of(&a.props).eq_ignore_ascii_case("CirclePawns"))
                .map(|(i, _)| i)
                .take(18)
                .collect();
            c.enter_tag("CirclePawns");
            for (k, i) in circle.into_iter().enumerate() {
                crate::fade::request_after(crate::fade::Who::Actor(i), 2.0, true, k as f32 * 2.0);
            }
        }
        "PlayDenouement" => {
            play(c, "01_Sols");
        }
        "SpawnGwog" => {
            crate::fade::request(crate::fade::Who::Tag("Beeblequix".into()), 2.0, false);
            crate::fade::request(crate::fade::Who::Tag("Pattie".into()), 2.0, false);
            c.enter_tag("Gwog");
            crate::fade::request(crate::fade::Who::Tag("Gwog".into()), 2.0, true);
        }
        "PlayDenouementEnd" => {
            play(c, "17_Final");
        }
        _ => return false,
    }
    true
}

impl Ctx<'_> {
    fn points_named(&self, tag: &str) -> Vec<DVec3> {
        self.lvl
            .actors
            .iter()
            .filter(|a| tag_of(&a.props).eq_ignore_ascii_case(tag))
            .map(|a| {
                let l = a.location();
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            })
            .collect()
    }
}
