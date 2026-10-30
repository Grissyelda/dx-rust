//! Starting a new game and moving between maps.
//!
//! Until maps can be swapped inside one process (the v1.0 level loader), going to another map starts a fresh
//! copy of the game on that map and hands it what it needs in `session.txt` beside the exe: the character
//! creation choices (DeusExPlayer.TruePlayerName, PlayerSkin, CombatDifficulty, the skill levels and points).
//! The new process reads and deletes the file at startup.

use std::sync::RwLock;

#[derive(Clone, Debug, PartialEq)]
pub struct NewGame {
    pub name: String,
    pub skin: u8,
    /// DeusExPlayer.CombatDifficulty: 1 Easy, 1.5 Medium, 2 Hard, 4 Realistic.
    pub difficulty: f32,
    /// (skill class, level).
    pub skills: Vec<(String, usize)>,
    pub points_avail: i32,
    pub points_total: i32,
    /// Lay D Denton (Human.bMadeFemale): the female JC. Only with the LDDP files laid over the game.
    pub female: bool,
    /// LDDP's "Original Morpheus?" (Human.bRetroMorpheus).
    pub retro_morpheus: bool,
    /// LDDP's "Original Interactions?" (Human.bFemaleUsesMaleInteractions).
    pub male_interactions: bool,
}

impl Default for NewGame {
    fn default() -> Self {
        NewGame {
            name: "JC Denton".into(),
            skin: 0,
            // Developer runs straight into a map play at Medium, as before.
            difficulty: 1.5,
            skills: Vec::new(),
            points_avail: 5000,
            points_total: 5000,
            female: false,
            retro_morpheus: false,
            male_interactions: false,
        }
    }
}

static SESSION: RwLock<Option<NewGame>> = RwLock::new(None);
static LOADED: std::sync::Once = std::sync::Once::new();

fn path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("session.txt")))
        .unwrap_or_else(|| "session.txt".into())
}

pub fn encode(ng: &NewGame) -> String {
    let mut s = format!(
        "name={}\nskin={}\ndifficulty={}\npoints_avail={}\npoints_total={}\n",
        ng.name.replace('\n', " "),
        ng.skin,
        ng.difficulty,
        ng.points_avail,
        ng.points_total
    );
    for (c, l) in &ng.skills {
        s.push_str(&format!("skill={c},{l}\n"));
    }
    s.push_str(&format!("female={}\nretro_morpheus={}\nmale_interactions={}\n", ng.female as u8, ng.retro_morpheus as u8, ng.male_interactions as u8));
    s
}

pub fn decode(t: &str) -> NewGame {
    let mut ng = NewGame::default();
    for line in t.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "name" => ng.name = v.to_string(),
            "skin" => ng.skin = v.parse().unwrap_or(0),
            "difficulty" => ng.difficulty = v.parse().unwrap_or(1.5),
            "points_avail" => ng.points_avail = v.parse().unwrap_or(5000),
            "points_total" => ng.points_total = v.parse().unwrap_or(5000),
            "female" => ng.female = v == "1",
            "retro_morpheus" => ng.retro_morpheus = v == "1",
            "male_interactions" => ng.male_interactions = v == "1",
            "skill" => {
                if let Some((c, l)) = v.split_once(',') {
                    ng.skills.push((c.to_string(), l.parse().unwrap_or(0)));
                }
            }
            _ => {}
        }
    }
    ng
}

fn load() {
    LOADED.call_once(|| {
        let p = path();
        if let Ok(t) = std::fs::read_to_string(&p) {
            let _ = std::fs::remove_file(&p);
            if let Ok(mut w) = SESSION.write() {
                *w = Some(decode(&t));
            }
        }
    });
}

/// The character this game was started with (None: started straight into a map, developer style).
pub fn session() -> Option<NewGame> {
    load();
    SESSION.read().ok().and_then(|s| s.clone())
}

/// The character of a loaded save (save.rs), in place of session.txt.
pub fn set_session(ng: NewGame) {
    load();
    if let Ok(mut w) = SESSION.write() {
        *w = Some(ng);
    }
}

/// The face (and Lay D Denton's choices) for the next map's build (the intro and title maps keep whatever was
/// chosen last).
pub fn set_look(skin: u8, female: bool, retro_morpheus: bool, male_interactions: bool) {
    load();
    if let Ok(mut w) = SESSION.write() {
        let s = w.get_or_insert_with(NewGame::default);
        s.skin = skin;
        s.female = female;
        s.retro_morpheus = retro_morpheus;
        s.male_interactions = male_interactions;
    }
}

static LDDP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// After the mod folders are set (main.rs): is the Lay D Denton Project among them (its FemJC package)?
pub fn init_lddp() {
    let has = unreal_pkg::paths::overlays().iter().any(|d| unreal_pkg::paths::find_ci(&d.join("System"), "FemJC.u").is_some());
    LDDP.store(has, std::sync::atomic::Ordering::Relaxed);
    if has {
        println!("dxview: Lay D Denton Project found");
    }
}

/// The Lay D Denton Project's files are laid over the game.
pub fn lddp() -> bool {
    LDDP.load(std::sync::atomic::Ordering::Relaxed)
}

/// Playing the female JC (LDDP's LDDPJCIsFemale).
pub fn female() -> bool {
    lddp() && session().is_some_and(|s| s.female)
}

/// LDDP's flags, set on every arrival (JCDentonMale.TravelPostAccept), all never expiring.
pub fn lddp_flags() -> [(&'static str, bool); 3] {
    let s = session().unwrap_or_default();
    [("LDDPJCIsFemale", female()), ("LDDPOGMorpheus", s.retro_morpheus), ("LDDPMaleCont4FJC", s.male_interactions)]
}

pub fn combat_difficulty() -> f32 {
    session().map_or(1.5, |s| s.difficulty)
}

/// DeusExPlayer.PlayerSkin: the portrait picked on the new-game screen (0..4).
pub fn skin() -> u8 {
    session().map_or(0, |s| s.skin.min(4))
}

/// JCDentonTex0 / 4 / 5 / 6 / 7 (JCDentonMale.TravelPostAccept); PaulDentonTex likewise.
pub(crate) fn skin_tex(base: &str) -> String {
    let n = match skin() {
        0 => 0,
        k => k as u32 + 3,
    };
    format!("DeusExCharacters.Skins.{base}Tex{n}")
}

/// Lay D Denton's skins for the female JC (JCDentonMale.TravelPostAccept): the face by PlayerSkin
/// (JCDentonFemaleTex0 / 4 / 5 / 6 / 7), then her coat, shirt, legs and glasses.
pub fn female_skins() -> [String; 8] {
    let n = match skin() {
        0 => 0,
        k => k as u32 + 3,
    };
    let f = |t: &str| format!("FemJC.{t}");
    [
        f(&format!("JCDentonFemaleTex{n}")),
        f("JCDentonFemaleTex2"),
        f("JCDentonFemaleTex3"),
        f("JCDentonFemaleTex0"),
        f("JCDentonFemaleTex1"),
        f("JCDentonFemaleTex2"),
        "DeusExCharacters.Skins.FramesTex4".into(),
        "DeusExCharacters.Skins.LensesTex5".into(),
    ]
}

/// JC's third-person mesh: GM_Trench, or GFM_Trench for Lay D Denton.
pub fn jc_mesh() -> String {
    // Augmentique: the outfit's body.
    if let Some(m) = crate::augmentique::player_mesh() {
        return m;
    }
    jc_base_mesh()
}

/// JC's mesh without an Augmentique outfit.
pub fn jc_base_mesh() -> String {
    if female() {
        return "DeusExCharacters.GFM_Trench".into();
    }
    // HDTP's JC (its HD characters switch on): the "hdtp" class layer has his mesh.
    for (c, n, _, v) in unreal_pkg::classes::override_layer("hdtp") {
        if c.eq_ignore_ascii_case("JCDentonMale") && n.eq_ignore_ascii_case("Mesh") {
            if let unreal_pkg::props::Value::Path(m) = v {
                return m;
            }
        }
    }
    "DeusExCharacters.GM_Trench".into()
}

/// The first-person hands for Lay D Denton (JCDentonMale.RenderOverlays): the weapons' WeaponHandsTex and the
/// mini crossbow's MiniCrossbowTex1 swap for her own, by PlayerSkin.
pub fn view_tex(path: &str) -> Option<String> {
    if !female() {
        return None;
    }
    let n = match skin() {
        0 => 0,
        k => k as u32 + 3,
    };
    let name = path.rsplit('.').next().unwrap_or(path);
    if name.eq_ignore_ascii_case("WeaponHandsTex") {
        Some(format!("FemJC.WeaponHandsTex{n}Fem"))
    } else if name.eq_ignore_ascii_case("MiniCrossbowTex1") {
        Some(format!("FemJC.MiniCrossbowTex{n}Fem"))
    } else {
        None
    }
}

/// Lay D Denton's body pictures in the HUD and persona screens (HUDHitDisplay, PersonaHealthBodyWindow,
/// PersonaAugmentationBodyWindow, PersonaScreenAugmentations, the medbot screens): FemJC.<name>Fem for her.
pub fn ui_tex(path: &str) -> Option<String> {
    const FEM: &[&str] = &[
        "HUDHitDisplay_Head",
        "HUDHitDisplay_Torso",
        "HUDHitDisplay_ArmLeft",
        "HUDHitDisplay_ArmRight",
        "HUDHitDisplay_LegLeft",
        "HUDHitDisplay_LegRight",
        "HUDHitDisplay_Body",
        "HealthBody_1",
        "HealthBody_2",
        "AugmentationsBody_1",
        "AugmentationsBody_2",
        "AugmentationsLocationCerebral",
        "AugmentationsLocationEyes",
        "AugmentationsLocationTorso",
        "AugmentationsLocationArms",
        "AugmentationsLocationLegs",
        "AugmentationsLocationSubdermal",
    ];
    if !female() {
        return None;
    }
    let name = path.rsplit('.').next().unwrap_or(path);
    FEM.iter().find(|f| f.eq_ignore_ascii_case(name)).map(|f| format!("FemJC.{f}Fem"))
}

/// The player's own cries (Human.PlayTakeHitSound, PlayDyingSound, Gasp; the soda's burp and the cigarettes'
/// cough): Lay D Denton's FemJC.FJC* in place of DeusExSounds.Player.Male*.
pub fn voice(male: &str) -> String {
    if female() {
        if let Some(rest) = male.strip_prefix("DeusExSounds.Player.Male") {
            return format!("FemJC.Player.FJC{rest}");
        }
    }
    male.to_string()
}

/// DeusExPlayer.UpdatePlayerSkin: Paul (PaulDenton / PaulDentonCarcass.SetSkin: MultiSkins 0 and 3), JC's carcass
/// and double (MultiSkins 0) wear the chosen face. Applied to the map's actors as it loads. Lay D Denton dresses
/// JC's carcass, double and (in the intro only) clone as her.
pub fn skin_overrides(map: &str) -> Vec<(String, String, u32, unreal_pkg::props::Value)> {
    use unreal_pkg::props::Value;
    let paul = skin_tex("PaulDenton");
    let jc = skin_tex("JCDenton");
    let mut v = Vec::new();
    for c in ["PaulDenton", "PaulDentonCarcass"] {
        for i in [0, 3] {
            v.push((c.to_string(), "MultiSkins".to_string(), i, Value::Path(paul.clone())));
        }
    }
    if !female() {
        for c in ["JCDentonMaleCarcass", "JCDouble"] {
            v.push((c.to_string(), "MultiSkins".to_string(), 0, Value::Path(jc.clone())));
        }
        return v;
    }
    let skins = female_skins();
    // JCDentonMaleCarcass.SetSkin keeps its GM_Trench_Carcass meshes; JCDouble.SetSkin becomes GFM_Trench.
    for c in ["JCDentonMaleCarcass", "JCDouble"] {
        for (i, t) in skins.iter().enumerate() {
            v.push((c.to_string(), "MultiSkins".to_string(), i as u32, Value::Path(t.clone())));
        }
    }
    v.push(("JCDouble".into(), "Mesh".into(), 0, Value::Path("DeusExCharacters.GFM_Trench".into())));
    v.push(("JCDouble".into(), "bIsFemale".into(), 0, Value::Bool(true)));
    v.push(("JCDouble".into(), "CarcassType".into(), 0, Value::Path("FemJC.JCDentonFemaleCarcass".into())));
    // DentonClone.SetSkin, in the intro (Area 51's clones stay men): her tube, on GFM_TShirtPants in the
    // Land pose, turned a quarter.
    if map.to_ascii_lowercase().starts_with("00_intro") {
        let c = "DentonClone";
        let tubes = [
            "FemJC.JCDentonFemaleTex0Tube",
            "DeusExItems.Skins.PinkMaskTex",
            "DeusExItems.Skins.PinkMaskTex",
            "DeusExItems.Skins.GrayMaskTex",
            "DeusExItems.Skins.BlackMaskTex",
            "DeusExItems.Skins.PinkMaskTex",
            "FemJC.DentonCloneTex2Fem",
            "FemJC.DentonCloneTex3Fem",
        ];
        for (i, t) in tubes.iter().enumerate() {
            v.push((c.into(), "MultiSkins".into(), i as u32, Value::Path(t.to_string())));
        }
        v.push((c.into(), "Mesh".into(), 0, Value::Path("DeusExCharacters.GFM_TShirtPants".into())));
        v.push((c.into(), "AnimSequence".into(), 0, Value::Name("Land".into())));
        v.push((c.into(), "AnimRate".into(), 0, Value::Float(0.0)));
    }
    v
}

/// JCDentonMale's defaults with the chosen face (the third-person JC in conversations and his death); Lay D
/// Denton's skins, mesh and carcass when she's the one playing.
pub fn jc_defaults(classes: &mut unreal_pkg::classes::ClassDb) -> Vec<unreal_pkg::props::Property> {
    let mut d = jc_base_defaults(classes);
    // Augmentique: the outfit he has on.
    crate::augmentique::dress_player(&mut d, false);
    d
}

/// jc_defaults without an Augmentique outfit (its "default" textures are these).
pub fn jc_base_defaults(classes: &mut unreal_pkg::classes::ClassDb) -> Vec<unreal_pkg::props::Property> {
    use unreal_pkg::props::{Property, Value};
    let mut d = classes.defaults("JCDentonMale");
    if female() {
        d.retain(|p| !["MultiSkins", "Mesh", "CarcassType"].iter().any(|n| p.name.eq_ignore_ascii_case(n)));
        for (i, t) in female_skins().into_iter().enumerate() {
            d.push(Property { name: "MultiSkins".into(), index: i as u32, value: Value::Path(t) });
        }
        d.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(jc_base_mesh()) });
        d.push(Property { name: "CarcassType".into(), index: 0, value: Value::Path("FemJC.JCDentonFemaleCarcass".into()) });
        return d;
    }
    // HDTP's JC (its own mesh, with his textures laid out its own way): his skins stay as HDTP sets them. The
    // original's face in slot 0 put his face on the back of HDTP's head.
    let mesh = jc_base_mesh();
    if !mesh.eq_ignore_ascii_case("DeusExCharacters.GM_Trench") {
        d.retain(|p| !p.name.eq_ignore_ascii_case("Mesh"));
        d.push(Property { name: "Mesh".into(), index: 0, value: Value::Path(mesh) });
        return d;
    }
    d.retain(|p| !(p.name.eq_ignore_ascii_case("MultiSkins") && p.index == 0));
    d.push(Property { name: "MultiSkins".into(), index: 0, value: Value::Path(skin_tex("JCDenton")) });
    d
}

/// The infolink portrait of a speaker (HUDInfoLinkDisplay: Paul has one per skin, _1 to _5).
pub fn portrait(speaker: &str) -> String {
    let mut p = format!("InfoPortraits.{}", &speaker[..speaker.len().min(16)]);
    if speaker == "PaulDenton" {
        p.push_str(&format!("_{}", skin() + 1));
    }
    p
}

pub fn player_name() -> String {
    session().map_or_else(|| "JC Denton".into(), |s| s.name)
}

/// Start a fresh game on `map` with these choices (closes this copy).
pub fn start(root: &std::path::Path, map: &str, ng: &NewGame) {
    let _ = std::fs::write(path(), encode(ng));
    relaunch(root, map);
}

/// Start a fresh copy of the game on `map`.
pub fn relaunch(root: &std::path::Path, map: &str) {
    if let Ok(exe) = std::env::current_exe() {
        if std::process::Command::new(exe).arg(root).arg(map).spawn().is_ok() {
            std::process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trips() {
        let ng = NewGame {
            name: "Agent Smith".into(),
            skin: 3,
            difficulty: 4.0,
            skills: vec![("SkillComputer".into(), 2), ("SkillWeaponPistol".into(), 1)],
            points_avail: 125,
            points_total: 5000,
            female: true,
            retro_morpheus: false,
            male_interactions: true,
        };
        assert_eq!(decode(&encode(&ng)), ng);
    }
}
