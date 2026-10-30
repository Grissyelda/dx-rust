//! ScriptedPawn AI: standing orders (Standing, Patrolling, Wandering, Sitting), pathfinding over the level's
//! NavigationPoints, and combat.
//!
//! Each pawn knows its alliance and how it feels about other alliances (`InitialAlliances`), sees with a
//! 160 degree field of view out to `SightRadius` with a line-of-sight trace, reacts after a short surprise
//! delay, turns at its `RotationRate`, closes in (along the path network when there is no direct line)
//! when the target is out of accurate range, and fires its `InitialInventory` weapon with Deus Ex's spread
//! (weapon BaseAccuracy + pawn BaseAccuracy). Pawns that are shot become hostile to whoever shot them
//! (`bHateShot`). When the enemy is lost they go back to their orders. Weapons are holstered outside
//! combat unless `bKeepWeaponDrawn`; two-handed weapons (Mass >= 30) use the *2H animation set.

use crate::actors::{Animated, Blockers, Deco, World};
use crate::movers::{HudMessage, Movers};
use crate::stealth::{Noise, NoiseKind, Noises};
use crate::weapons::{
    spawn_mesh_parts, spread_dir, weapon_stats, Clip, LastAttacker, PawnHealth, RuntimeAssets,
    WeaponStats,
};
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::prelude::*;
use dx_level::actors::rotator_to_bevy_mat3;
use dx_level::collision::{trace_cylinder, Cylinder};
use dx_level::ue_to_bevy;
use glam::DVec3;
use imsim_core::movement::{Character, CollisionWorld, MoveParams, Physics};
use unreal_pkg::props::{find, Property, Value};

/// Deus Ex "Medium" combat difficulty: damage NPCs deal to the player is multiplied by this.
/// DeusExPlayer.CombatDifficulty (chosen at New Game; see travel.rs).

/// Player body parts (DeusExPlayer HealthHead, HealthTorso, HealthArmLeft/Right, HealthLegLeft/Right).
pub const P_HEAD: usize = 0;
pub const P_TORSO: usize = 1;
pub const P_ARM_L: usize = 2;
pub const P_ARM_R: usize = 3;
pub const P_LEG_L: usize = 4;
pub const P_LEG_R: usize = 5;

/// The game's flags (FlagBase).
#[derive(Resource, Default)]
pub struct GameFlags(pub imsim_core::flags::FlagBase);

/// Every takedown so far: lethal (killed) or non-lethal (knocked out: riot prod, tranquilliser darts, baton, gas
/// and so on), and whether the player did it. For the reactions and story beats that care how JC works.
#[derive(Resource, Default)]
pub struct Takedowns {
    pub log: Vec<Takedown>,
}

#[derive(Debug, Clone)]
pub struct Takedown {
    pub name: String,
    /// The pawn's alliance (NSF, UNATCO...).
    pub alliance: String,
    pub class: String,
    pub lethal: bool,
    pub by_player: bool,
    pub damage_type: &'static str,
}

impl Takedowns {
    pub fn count(&self, lethal: bool, by_player: bool) -> usize {
        self.log
            .iter()
            .filter(|t| t.lethal == lethal && t.by_player == by_player)
            .count()
    }
}

/// Debug god mode (spawn menu): the player takes no damage at all.
pub static GOD_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Resource)]
pub struct PlayerHealth {
    /// Overall health (GenerateTotalHealth).
    pub hp: f32,
    pub parts: [f32; 6],
    pub dead: bool,
    /// Flash when hit, fades out, coloured by damage type (PlayDXTakeDamageHit's ClientFlash fog).
    pub flash: f32,
    pub flash_color: [f32; 3],
    /// DeusExPlayer poison: damage every 2 s, `poison_counter` more times.
    pub poison_counter: u32,
    pub poison_timer: f32,
    pub poison_damage: f32,
    /// drugEffectTimer: seconds of the poisoned/drunk screen effect left.
    pub drug_timer: f32,
    /// Hazmat suit / ballistic armor running (charged.rs), and the charge readout for the HUD.
    pub hazmat: bool,
    pub armor: bool,
    /// Thermoptic camo running: CalculatePlayerVisibility returns 0 for every pawn.
    pub camo: bool,
    /// Bioelectric energy (augmentations drain it, bio cells top it up).
    pub energy: f32,
    /// DeusExPlayer.CatchFire: seconds alight (None = not), and the 1 s Burned timer.
    pub on_fire: Option<f32>,
    pub burn_tick: f32,
    pub charged_text: String,
    /// The body (Region) is in a water zone: pain sounds change (Human.PlayTakeHitSound).
    pub in_water: bool,
    /// swimTimer / swimDuration while swimming, for the HUD.
    pub breath: Option<f32>,
    /// Hits this frame for the HUD's damage display (AddDamageDisplay): damage type and hit offset.
    pub damage_icons: Vec<(String, DVec3)>,
    /// SetDamagePercent: share of the last hit absorbed by charged pickups.
    pub absorb: Option<f32>,
    /// EMP hits: energy still to drain and the per-second rate (EnergyDrain / EnergyDrainTotal).
    pub energy_drain: f32,
    pub energy_drain_total: f32,
}

impl Default for PlayerHealth {
    fn default() -> Self {
        PlayerHealth {
            hp: 100.0,
            parts: [100.0; 6],
            dead: false,
            flash: 0.0,
            flash_color: [0.6, 0.0, 0.0],
            poison_counter: 0,
            poison_timer: 0.0,
            poison_damage: 0.0,
            drug_timer: 0.0,
            hazmat: false,
            armor: false,
            camo: false,
            energy: 100.0,
            on_fire: None,
            burn_tick: 0.0,
            charged_text: String::new(),
            in_water: false,
            breath: None,
            damage_icons: Vec::new(),
            absorb: None,
            energy_drain: 0.0,
            energy_drain_total: 0.0,
        }
    }
}

/// Player collision size (JCDentonMale) for hit locations.
const PLAYER_RADIUS: f64 = 20.0;
const PLAYER_HEIGHT: f64 = 47.5;

impl PlayerHealth {
    /// GenerateTotalHealth after the parts were changed directly (the health screen).
    pub fn recompute(&mut self) {
        self.total();
    }

    /// GenerateTotalHealth: the average of the limbs and the average of head and torso (0 if either is gone).
    fn total(&mut self) {
        let p = &self.parts;
        let ave = (p[P_LEG_L] + p[P_LEG_R] + p[P_ARM_L] + p[P_ARM_R]) / 4.0;
        let crit = if p[P_HEAD] <= 0.0 || p[P_TORSO] <= 0.0 {
            0.0
        } else {
            (p[P_HEAD] + p[P_TORSO]) / 2.0
        };
        self.hp = if crit == 0.0 { 0.0 } else { (ave + crit) / 2.0 };
        if self.hp <= 0.0 {
            self.hp = 0.0;
            self.dead = true;
        }
    }

    /// DeusExPlayer.TakeDamage / DXReduceDamage / HandleDamage: `offset` is the hit point relative to the player in
    /// his own frame (X forward, Y right, Z up). Gunshots are scaled by the combat difficulty; EMP only drains
    /// bioelectric energy.
    pub fn damage(&mut self, dmg: f32, damage_type: &str, offset: DVec3) {
        // DXVIEW_GOD=1: god mode from the start (test runs).
        static ENV_GOD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if self.dead || GOD_MODE.load(std::sync::atomic::Ordering::Relaxed) || *ENV_GOD.get_or_init(|| std::env::var("DXVIEW_GOD").is_ok()) {
            return;
        }
        self.damage_icons.push((damage_type.to_string(), offset));
        if damage_type == "NanoVirus" {
            return;
        }
        // EMP drains bioelectric energy (EnergyDrain, spent over about a second), less with the EMP Shield.
        if damage_type == "EMP" {
            let d = dmg * crate::abilities::aug("AugEMP").unwrap_or(1.0);
            self.energy_drain += d;
            self.energy_drain_total += d;
            return;
        }
        let mut d = dmg;
        if matches!(damage_type, "Shot" | "AutoShot") {
            d *= crate::travel::combat_difficulty();
            if d <= 1.0 && dmg > 0.0 {
                d = 1.0;
            }
        }
        if damage_type == "Poison" {
            self.start_poison(dmg);
        }
        if damage_type == "Flamed" && self.on_fire.is_none() {
            self.on_fire = Some(0.0);
            self.burn_tick = 0.0;
        }
        // DXReduceDamage: augmentations, and charged pickups scaled by Environmental Training.
        let gas = matches!(
            damage_type,
            "TearGas" | "PoisonGas" | "Radiation" | "HalonGas" | "PoisonEffect" | "Poison"
        );
        let enviro = crate::abilities::skill("SkillEnviro");
        if gas {
            if let Some(v) = crate::abilities::aug("AugEnviro") {
                d *= v;
            }
            if self.hazmat {
                d *= 0.75 * enviro;
            }
        }
        if self.armor && matches!(damage_type, "Shot" | "Sabot" | "Exploded" | "AutoShot") {
            d *= 0.5 * enviro;
        }
        if matches!(damage_type, "Shot" | "AutoShot") {
            if let Some(v) = crate::abilities::aug("AugBallistic") {
                d *= v;
            }
        }
        if matches!(damage_type, "Burned" | "Flamed" | "Exploded" | "Shocked") {
            if let Some(v) = crate::abilities::aug("AugShield") {
                d *= v;
            }
        }
        // Fully protected from gas: the poison stops (StopPoison) and the drunk effect shortens.
        if gas && d <= 0.0 && dmg > 0.0 {
            self.poison_counter = 0;
            self.drug_timer = (self.drug_timer - 4.0).max(0.0);
        }
        if dmg > 0.0 && d < dmg {
            self.absorb = Some(1.0 - d / dmg);
        }
        let d = d.floor().max(if dmg > 0.0 { 1.0 } else { 0.0 });
        let head_z = PLAYER_HEIGHT * 0.78;
        let head_y = PLAYER_RADIUS * 0.35;
        let arm = PLAYER_RADIUS * 0.35;
        let p = &mut self.parts;
        if offset.z > head_z {
            // Only the narrow head region counts.
            if offset.x.abs() < head_y || offset.y.abs() < head_y {
                p[P_HEAD] -= d * 2.0;
            }
        } else if offset.z < 0.0 {
            if offset.y > 0.0 {
                p[P_LEG_R] -= d;
            } else {
                p[P_LEG_L] -= d;
            }
            // A dead leg passes damage to the other, then to the torso.
            if p[P_LEG_R] < 0.0 && p[P_LEG_L] > 0.0 {
                p[P_LEG_L] += p[P_LEG_R];
                p[P_LEG_R] = 0.0;
            } else if p[P_LEG_L] < 0.0 && p[P_LEG_R] > 0.0 {
                p[P_LEG_R] += p[P_LEG_L];
                p[P_LEG_L] = 0.0;
            }
            for leg in [P_LEG_L, P_LEG_R] {
                if p[leg] < 0.0 {
                    p[P_TORSO] += p[leg];
                    p[leg] = 0.0;
                }
            }
        } else {
            if offset.y > arm {
                p[P_ARM_R] -= d;
            } else if offset.y < -arm {
                p[P_ARM_L] -= d;
            } else {
                p[P_TORSO] -= d * 2.0;
            }
            for a in [P_ARM_L, P_ARM_R] {
                if p[a] < 0.0 {
                    p[P_TORSO] += p[a];
                    p[a] = 0.0;
                }
            }
        }
        self.flash = 0.35;
        if dmg > 0.0 {
            crate::shake::shake_view(0.15 + 0.002 * dmg, dmg * 30.0, 0.3 * dmg);
            let side = (offset.y as f32 / arm.max(1.0) as f32).clamp(-1.0, 1.0);
            crate::dualsense::event(crate::dualsense::Haptic::Hurt((dmg / 40.0).min(1.0), side));
        }
        // ClientFlash fog by damage type: burns reddish, radiation yellow-brown, poison gas green, tear gas yellow,
        // drowning blue, everything else dark red.
        self.flash_color = match damage_type {
            "Burned" | "Flamed" => [0.8, 0.4, 0.4],
            "Radiation" => [0.5, 0.5, 0.0],
            "PoisonGas" => [0.2, 0.6, 0.0],
            "TearGas" => [0.6, 0.6, 0.0],
            "Drowned" => [0.0, 0.4, 0.8],
            _ => [0.6, 0.0, 0.0],
        };
        let was_dead = self.dead;
        self.total();
        // Human.PlayDyingSound / PlayTakeHitSound (at most one cry every 0.5..1.5 s).
        if self.dead && !was_dead {
            crate::sound::play_owned(
                crate::sound::PLAYER,
                crate::sound::slot::PAIN,
                // Human.PlayDyingSound: MaleWaterDeath in water (Lay D Denton: her own, travel::voice).
                &crate::travel::voice(if self.in_water { "DeusExSounds.Player.MaleWaterDeath" } else { "DeusExSounds.Player.MaleDeath" }),
                None,
                1.0,
                0.0,
                crate::sound::random_pitch(),
            );
        } else if !self.dead && d > 0.0 && self.in_water {
            // Human.PlayTakeHitSound in water: drowning gurgles (80%), anything else a small grunt.
            let snd = if damage_type == "Drowned" {
                (crate::sound::frand() < 0.8).then_some("DeusExSounds.Player.MaleDrown")
            } else {
                Some("DeusExSounds.Player.MalePainSmall")
            };
            if let Some(snd) = snd {
                crate::sound::play_req(crate::sound::Req {
                    sound: crate::travel::voice(snd),
                    pos: None,
                    volume: 2.0,
                    radius: 0.0,
                    pitch: crate::sound::random_pitch(),
                    owner: crate::sound::PLAYER,
                    slot: crate::sound::slot::PAIN,
                    no_override: false,
                    min_gap: 0.5 + crate::sound::frand(),
                });
            }
        } else if !self.dead && d > 0.0 && damage_type != "Drowned" {
            let snd = match damage_type {
                "TearGas" | "HalonGas" => "DeusExSounds.Player.MaleEyePain".to_string(),
                "PoisonGas" => "DeusExSounds.Player.MaleCough".to_string(),
                _ => [
                    "DeusExSounds.Player.MalePainSmall",
                    "DeusExSounds.Player.MalePainMedium",
                    "DeusExSounds.Player.MalePainLarge",
                ][((crate::sound::frand() * 3.0) as usize).min(2)]
                .to_string(),
            };
            crate::sound::play_req(crate::sound::Req {
                sound: crate::travel::voice(&snd),
                pos: None,
                volume: 2.0,
                radius: 0.0,
                pitch: crate::sound::random_pitch(),
                owner: crate::sound::PLAYER,
                slot: crate::sound::slot::PAIN,
                no_override: false,
                min_gap: 0.5 + crate::sound::frand(),
            });
        }
    }

    /// HealPlayer: points go to the head first, then torso, right leg, left leg, right arm, left arm, each up to 100.
    pub fn heal(&mut self, n: f32) -> f32 {
        if n < 0.0 {
            // HealPlayer with a negative amount (zyme): taken off the parts in the same order.
            let mut left = -n;
            for i in [P_HEAD, P_TORSO, P_LEG_R, P_LEG_L, P_ARM_R, P_ARM_L] {
                let take = self.parts[i]
                    .max(0.0)
                    .min(left)
                    .min(self.parts[i] - 1.0)
                    .max(0.0);
                self.parts[i] -= take;
                left -= take;
            }
            self.total();
            return n + left;
        }
        let mut left = n;
        for i in [P_HEAD, P_TORSO, P_LEG_R, P_LEG_L, P_ARM_R, P_ARM_L] {
            let room = (100.0 - self.parts[i]).max(0.0);
            let take = room.min(left);
            self.parts[i] += take;
            left -= take;
        }
        self.total();
        n - left
    }

    /// Gas reaching the player.
    pub fn gas(&mut self, damage_type: &str, damage: f32) {
        if damage_type == "TearGas" {
            self.drug_timer = self.drug_timer.max(1.0);
        }
        self.damage(damage, damage_type, DVec3::ZERO);
    }

    /// DeusExPlayer.StartPoison: four more doses two seconds apart, and four more seconds of the drug effect.
    pub fn start_poison(&mut self, dmg: f32) {
        if self.dead {
            return;
        }
        // GMDX Hardened: three doses instead of four and no blurred view (drugEffectTimer = 0).
        let hardened = crate::perks::has("PerkHardened");
        self.poison_counter = if hardened { 3 } else { 4 };
        self.poison_timer = 0.0;
        self.poison_damage = self.poison_damage.max(dmg);
        if hardened {
            self.drug_timer = 0.0;
        } else {
            self.drug_timer += 4.0;
        }
    }

    /// StopPoison (GMDX Toxicologist medkits): no more doses, no drug effect.
    pub fn cure_poison(&mut self) {
        self.poison_counter = 0;
        self.poison_timer = 0.0;
        self.poison_damage = 0.0;
        self.drug_timer = 0.0;
    }

    /// Movement speed multiplier from wounded legs and torso (PlayerWalking.ProcessMove), and whether both legs are
    /// gone (crawl: 0.8 speed, forced crouch).
    pub fn speed_factor(&self) -> (f64, bool) {
        let p = &self.parts;
        if p[P_LEG_L] < 1.0 && p[P_LEG_R] < 1.0 {
            return (0.8, true);
        }
        // GMDX Perserverance: wounds don't slow the player (only losing both legs does).
        if crate::perks::has("PerkPerserverance") {
            return (1.0, false);
        }
        let mut k = 1.0f64;
        for leg in [P_LEG_L, P_LEG_R] {
            k -= 0.5
                * if p[leg] < 1.0 {
                    0.25
                } else if p[leg] < 34.0 {
                    0.15
                } else if p[leg] < 67.0 {
                    0.10
                } else {
                    0.0
                };
        }
        if p[P_TORSO] < 67.0 {
            k -= 0.5 * 0.05;
        }
        if p[P_LEG_L] < 1.0 && p[P_LEG_R] < 1.0 {
            return (0.8, true);
        }
        (k, false)
    }

    /// CalculateAccuracy penalties from wounded arms and head.
    pub fn accuracy_penalty(&self) -> f32 {
        self.accuracy_penalty_with(false)
    }

    /// With `best_arm` (GMDX Ambidextrous perk, pistols) only the healthier arm counts.
    pub fn accuracy_penalty_with(&self, best_arm: bool) -> f32 {
        let p = &self.parts;
        let mut a = 0.0;
        let pen = |h: f32| if h < 1.0 {
            0.5
        } else if h < 34.0 {
            0.2
        } else if h < 67.0 {
            0.1
        } else {
            0.0
        };
        if best_arm {
            a += pen(p[P_ARM_R].max(p[P_ARM_L]));
        } else {
            for arm in [P_ARM_R, P_ARM_L] {
                a += pen(p[arm]);
            }
        }
        if p[P_HEAD] < 67.0 {
            a += 0.1;
        }
        a
    }
}

/// A world point relative to the player, in his own frame (X forward, Y right, Z up).
pub fn player_offset(player_center: DVec3, view_yaw: f64, at: DVec3) -> DVec3 {
    let d = at - player_center;
    let (s, c) = view_yaw.sin_cos();
    DVec3::new(d.x * c + d.y * s, -d.x * s + d.y * c, d.z)
}

#[derive(Component)]
pub struct HealthText;
#[derive(Component)]
pub struct DamageFlash;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NpcState {
    /// Following orders.
    Idle,
    /// Spotted an enemy; reacts when the timer runs out.
    Surprised(f32),
    Attacking,
    /// Playing a hit reaction (ScriptedPawn TakingHit); seconds left.
    TakingHit(f32),
    /// Riot prod / stun weapons (ScriptedPawn Stunned): twitching for this many more seconds.
    Stunned(f32),
    /// Tear gas / pepper spray (ScriptedPawn RubbingEyes): RubEyesStart, RubEyes for 15 s, RubEyesStop; seconds left.
    RubbingEyes(f32),
    /// Investigating something seen, heard or lost (ScriptedPawn Seeking); details in `Npc::seek`.
    Seeking,
    /// On fire (ScriptedPawn Burning): panics and runs about until the fire goes out.
    Burning,
    /// ScriptedPawn Fleeing: badly hurt (Health <= MinHealth or a limb gone) or out of ammo; runs for cover until
    /// the enemy has been out of sight for FearSustainTime.
    Fleeing,
    /// ScriptedPawn Alerting: running to an alarm unit to push it (RaiseAlarm before attacking or before fleeing).
    Alerting,
    Dead,
}

/// Why a pawn is seeking (ScriptedPawn.ESeekType).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeekKind {
    /// Glimpsed an enemy (EnemyReadiness passed SightPercentage): turn and look.
    Sight,
    /// Heard one (LoudNoise): go and look.
    Sound,
    /// Lost one in a fight (post-combat search).
    Guess,
    /// Found a body (CheckCarcassPresence): go and look at it.
    Carcass,
}

/// Where in the Seeking state script the pawn is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeekPhase {
    /// Begin: PickDestination.
    Begin,
    /// GoToLocation: walking/running toward `dest`.
    Go,
    /// Guess searches run on a bit past where the enemy was.
    Overshoot(DVec3),
    /// TurnToLocation.
    Turn,
    /// Sleep(3..4.5 s) after turning.
    Wait(f32),
    /// LookAround (cut short when something new turns up).
    Look(f32),
}

#[derive(Debug, Clone)]
pub struct Seek {
    pub kind: SeekKind,
    /// LastSeenPos.
    pub pos: DVec3,
    /// destLoc.
    pub dest: DVec3,
    /// bSeekLocation: still heading for `pos` (as opposed to poking around nearby).
    pub located: bool,
    pub phase: SeekPhase,
    pub post_combat: bool,
}

/// What a pawn does when nothing is going on (ScriptedPawn.Orders).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Orders {
    Standing,
    Patrolling,
    Wandering,
    Sitting,
    /// RunningTo / GoingTo: to the OrderTag actor (`order_goal`), or the player, then Standing there.
    RunningTo,
    GoingTo,
    /// WaitingFor: stand until the player is in sight, then run to him.
    WaitingFor,
}

/// A target: the player or another pawn (blocker index).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Player,
    Pawn(usize),
}

/// A path being followed: nav point indices still to visit, then the goal itself.
#[derive(Debug, Clone, Default)]
pub struct Route {
    pub goal: DVec3,
    pub nodes: Vec<usize>,
    /// Re-plan after this many seconds (targets move).
    pub age: f32,
    /// Progress watchdog.
    pub check_pos: DVec3,
    pub check_time: f32,
    pub stuck: u32,
    /// Planned over the path network even though a straight line looked clear.
    pub forced: bool,
    /// The straight line turned out to be blocked; plan over the network instead.
    pub needs_graph: bool,
}

/// Attacking's labels, as far as firing goes (see Npc::atk_phase).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AtkPhase {
    /// RunToRange: TurnToward(enemy) before moving or firing (bCanFire false).
    TurnFirst,
    /// RunToRange's move loop (bCanFire true: strafers fire facing the enemy, the rest within FireAngle).
    Moving,
    /// Fire: stopped, TweenToShoot(0.15) and TurnToward before ContinueFire.
    Settle(f32),
    /// ContinueFire: a shot whenever the weapon is ready, turning toward the enemy between shots.
    Firing,
}

/// ScriptedPawn state OpeningDoor.
#[derive(Clone, Debug)]
pub struct DoorWait {
    pub mover: usize,
    /// FindBackupPoint's destLoc (or where we stand).
    pub back_to: DVec3,
    pub t: f32,
    /// FrobDoor done (Frob before backing up when the door doesn't encroach, after it when it does).
    pub frobbed: bool,
    pub encroaches: bool,
}

#[derive(Component)]
pub struct Npc {
    pub actor: usize,
    pub blocker: usize,
    pub alliance: String,
    /// (alliance, level -1..1): below -0.5 is an enemy, as ScriptedPawn.GetAllianceType reads it.
    /// AlliancesEx (imsim_core::alliance): feelings toward other alliances, agitation from provocations.
    pub alliances: imsim_core::alliance::AllianceTable,
    pub state: NpcState,
    pub target: Option<Target>,
    pub last_seen: f32,
    pub last_pos: DVec3,
    pub yaw: f64,
    pub body: Option<Character>,
    pub params: MoveParams,
    pub weapon: Option<WeaponStats>,
    pub inited: bool,
    pub loaded: u32,
    pub shot_timer: f32,
    pub reload_timer: f32,
    pub sight_radius: f64,
    /// bTickVisibleOnly: away from the player and not drawn lately, other pawns go unchecked (ScriptedPawn.Tick:
    /// bCheckOther false beyond 600 when LastRendered() >= 5, bCheckPlayer false beyond 2500) - NPC fights
    /// wait for the player to see them (Battery Park's terrorists vs the troops on the wall, the intro).
    pub tick_visible: bool,
    /// Game time this pawn was last on screen (LastRenderTime), checked every 0.2 s.
    pub last_rendered: f32,
    pub render_check: f32,
    pub fov_cos: f64,
    pub base_accuracy: f32,
    pub surprise: f32,
    pub enemy_timeout: f32,
    pub yaw_rate: f64,
    pub anim: &'static str,
    pub think: f32,
    pub seed: u64,
    // Orders.
    pub orders: Orders,
    pub order_tag: String,
    /// Where RunningTo / GoingTo head (None: the player).
    pub order_goal: Option<DVec3>,
    pub walk_frac: f64,
    pub walk_anim_mult: f32,
    pub run_anim_mult: f32,
    pub restlessness: f32,
    pub wanderlust: f32,
    /// Standing CollisionHeight; crouching shrinks it to 0.65 of that (StartCrouch / SetBasedPawnSize).
    pub stand_height: f64,
    /// A melee swing under way: seconds until its HandToHandAttack notify lands the blow.
    pub melee_wait: Option<f32>,
    pub keep_drawn: bool,
    /// The weapon is out (drawn) right now.
    pub weapon_drawn: bool,
    pub home: DVec3,
    pub home_yaw: f64,
    /// Home base (ScriptedPawn.InitializeHomeBase): HomeTag 'Start' (the default) makes where it was placed home;
    /// another tag looks for that HomeBase actor (its location, rotation and Extent). Wandering stays within
    /// HomeExtent of it (and in sight of it); bDefendHome also keeps combat moves inside.
    pub use_home: bool,
    pub home_extent: f64,
    pub defend_home: bool,
    /// Animals wander to random spots (AIPickRandomDestination) instead of path nodes.
    pub animal: bool,
    pub wander_goal: Option<DVec3>,
    pub patrol_dest: Option<usize>,
    /// Waiting at a patrol/wander point: seconds left and the direction to face.
    pub pause: f32,
    pub pause_yaw: Option<f64>,
    pub route: Option<Route>,
    pub sitting: bool,
    /// The seat sat on (Sitting: SetBase(SeatActor)): its actor, the blocker found for it (looked up once), and
    /// where the pawn's cylinder centre sits relative to the seat's - the pawn follows the seat as it settles or
    /// is pushed, as a based actor does.
    pub seat: Option<(usize, Option<usize>, DVec3)>,
    pub orders_failed: bool,
    /// Per-body-part health.
    pub hp_body: crate::damage::Body,
    /// No new pain animation until this runs out (ScriptedPawn.TakeHitTimer).
    pub take_hit_timer: f32,
    /// Knocked out rather than killed.
    pub knocked_out: bool,
    /// Carcass has been searched (its inventory taken).
    pub searched: bool,
    /// A shot went off this frame (play the shoot animation once).
    pub just_fired: bool,
    // Senses (ScriptedPawn CheckEnemyPresence / CheckCycle / UpdateReactionLevel).
    /// EnemyReadiness 0..1: >= SightPercentage starts a sight search, 1 means attack.
    pub readiness: f32,
    /// ReactionLevel 0..1: how ready to act once an enemy is confirmed (the surprise pause is what's left of it).
    pub reaction: f32,
    /// CycleTimer: seconds readiness holds after losing sight before decaying.
    pub sustain: f32,
    /// CyclePeriod: time since the last perception cycle.
    pub cycle: f32,
    pub vis_threshold: f32,
    pub hear_threshold: f32,
    pub sight_pct: f32,
    pub react_presence: bool,
    pub react_loud: bool,
    pub react_shot: bool,
    pub react_distress: bool,
    pub react_alarm: bool,
    pub react_carcass: bool,
    pub hate_carcass: bool,
    pub fear_carcass: bool,
    pub react_projectiles: bool,
    /// The rest of the bReact / bHate / bFear flags behind the reaction callbacks (UpdateReactionCallbacks).
    pub react_futz: bool,
    pub hate_shot: bool,
    pub fear_shot: bool,
    pub hate_weapon: bool,
    pub fear_weapon: bool,
    pub hate_hacking: bool,
    pub fear_hacking: bool,
    pub fear_alarm: bool,
    pub fear_distress: bool,
    pub fear_injury: bool,
    pub fear_indirect_injury: bool,
    pub hate_indirect_injury: bool,
    pub fear_projectiles: bool,
    /// FearLevel / FearTimer: civilians run once something scares them enough (IsFearful).
    pub fear: imsim_core::reactions::Fear,
    /// FutzTimer: at most one futz bark every 2 s.
    pub futz_timer: f32,
    /// The current flight is from fear (the enemy needn't be hostile; it ends when no longer fearful).
    pub fear_flee: bool,
    /// bCower: fleeing pawns crouch and cover up where they are (CowerBegin / Cowering / CowerEnd).
    pub cower: bool,
    /// Seconds to the next sight check of continuous stimuli (a drawn weapon, a tool at work).
    pub stim_check: f32,
    pub avoid_harm: bool,
    /// RaiseAlarm: 0 before attacking, 1 before fleeing, 2 never.
    pub raise_alarm: u8,
    /// AlarmTimer: after an alarm (pushed or heard) don't go for another for 120 s.
    pub alarm_timer: f32,
    /// HandlingEnemy should consider Alerting first.
    pub alert_pending: bool,
    /// Enhancement (Bodies Raise Alarms): the body that sent this pawn to the alarm; it searches there after.
    pub body_alarm: Option<DVec3>,
    /// Alerting: (alarm index, seconds of the push animation left once there).
    pub alert: Option<(usize, f32)>,
    /// Bodies already reacted to (Carcasses[4], by blocker), and CarcassCheckTimer.
    pub carcasses_seen: Vec<usize>,
    pub carcass_check: f32,
    /// PotentialEnemyAlliance / PotentialEnemyTimer (bHateCarcass): the killers' alliance, hated on sight.
    pub potential_enemy: Option<(String, f32)>,
    /// For our body: KillerAlliance, and bEmitCarcass (killed, not knocked out, not a robot).
    pub killer_alliance: String,
    /// DeusExCarcass.KillerBindName: who killed it (the player is "JCDenton").
    pub killer_bind: String,
    /// GuntherHermann KillswitchActivated: seconds into the convulsions.
    pub killswitch: Option<f32>,
    pub emit_carcass: bool,
    /// AvoidingProjectiles: running from a live grenade until this many seconds pass.
    pub avoiding: f32,
    /// Bleeding (bCanBleed): bleedRate rises with damage taken and clots over ClotPeriod; drops fall more
    /// often the faster the pawn moves.
    pub can_bleed: bool,
    /// MedicalBot / RepairBot: seconds until it can serve again, and seconds left standing still with its arm out
    /// after serving (StandStill, 'Start' anim; then 'Stop' and back to orders).
    pub service_cooldown: f32,
    pub service_pause: f32,
    /// Animal.bFleeBigPawns: startled by big pawns heading its way (checked every 0.5 s while not fighting), it
    /// keeps stepping ~100 units clear of where they'll be (AvoidingPawn).
    pub flee_big: bool,
    /// CleanerBot: its Wandering sweeps the floor in a lawnmower pattern (PickDestination: 256 units along the minor
    /// direction, a step of 2 x CollisionRadius along the major one when blocked, each reversed when it fails),
    /// it wipes up decals under it every 0.3 s and it gets out of the way of pawns heading at it (AvoidingPawn).
    pub cleaner: bool,
    pub minor_dir: u8,
    pub major_dir: u8,
    /// Seconds spent on the current sweep leg (a leg blocked by a pawn is given up after 8 s).
    pub clean_time: f32,
    /// Stall watchdog: seconds wanting to move without moving, and legs given up in a row.
    pub stall: f32,
    /// bCanOpenDoors (animals can't).
    pub can_open_doors: bool,
    /// State OpeningDoor: the door (mover index) being opened, where to back up to, and the time spent.
    pub door: Option<DoorWait>,
    pub stalls: u32,
    pub startle_timer: f32,
    /// Rat: every second, a 5% chance to squeak.
    pub squeak_timer: f32,
    pub service_shown: bool,
    /// bCanCrouch / CrouchRate: on stopping to fire at a distant enemy, the pawn may drop to a crouch
    /// (CrouchShoot, 65% height) for at least 1-1.5 s.
    pub can_crouch: bool,
    /// Attacking's fire rules: bCanStrafe (runs facing the enemy and keeps firing), bMustFaceTarget (no shots on the
    /// move otherwise), FireAngle (else: fires on the move when the enemy is within FireAngle/2 degrees of the way it
    /// faces - robots, 60).
    pub can_strafe: bool,
    /// bCanSwim: a burning pawn that can swim makes for water within 2000 and is put out when it reaches it.
    pub can_swim: bool,
    pub must_face: bool,
    pub fire_angle: f32,
    /// Where Attacking's state code is: RunToRange's opening TurnToward, running to range, the Fire label's
    /// TweenToShoot (0.15 s) + TurnToward before the first shot, or ContinueFire.
    pub atk_phase: AtkPhase,
    pub crouch_rate: f32,
    pub crouching: bool,
    pub crouch_timer: f32,
    /// Whether this stop to fire already rolled for a crouch.
    pub crouch_rolled: bool,
    /// BindName and bImportant: important pawns record <BindName>_Dead / _Unconscious flags when taken down.
    pub bind_name: String,
    pub important: bool,
    pub bleed_rate: f32,
    pub drop_counter: f32,
    pub clot_period: f32,
    pub hate_distress: bool,
    /// bHateInjury: being hurt is a provocation (IncreaseAgitation).
    pub hate_injury: bool,
    pub seek: Option<Seek>,
    /// SeekLevel: how many more places to search.
    pub seek_level: i32,
    /// Ammo carried for the weapon (InitialInventory), which picks the projectile for dart guns.
    pub ammo: Option<String>,
    /// ScriptedPawn poison: `poison_counter` more doses of PoisonEffect every 2 s.
    pub poison_counter: u32,
    pub poison_timer: f32,
    pub poison_damage: f32,
    pub poisoner: Option<usize>,
    /// GMDX Hemorrhage (StartBleed): `bleed_counter` more BleedEffect ticks of `bleed_damage`, one a second.
    pub bleed_counter: u32,
    pub bleed_timer: f32,
    pub bleed_damage: f32,
    /// This body is in the player's hands (POVCorpse).
    pub carried: bool,
    /// Robot.EMPHitPoints.
    pub emp_hp: f32,
    /// Every weapon carried (InitialInventory order) and the ammo left by class; `weapon` is the one in hand.
    pub arsenal: Vec<WeaponStats>,
    pub ammo_left: Vec<(String, i32)>,
    /// WeaponTimer: seconds since the last weapon change (sticks with a new choice for 5-10 s).
    pub weapon_timer: f32,
    pub switch_check: f32,
    /// ScriptedPawn.FireTimer: set to the weapon's AIFireDelay when it fires, counts down every tick.
    pub fire_timer: f32,
    /// Killed with no death animation: replace the pawn with its CarcassType's mesh (carcass_swap_system).
    pub swap_carcass: Option<String>,
    /// Robot.CrazedTimer: seconds of reversed alliances left (scramble grenade).
    pub crazed: f32,
    /// Died in water: the floating carcass (CarcassType's Mesh3) takes the body's place once it has fallen.
    pub water_carcass: Option<(String, f32)>,
    /// That floating body's entity (taken away with the body when it's picked up).
    pub float_body: Option<Entity>,
    /// Gray.Tick radiation aura (damageRadius, damageInterval, damageAmount) and its timer (damageTime).
    pub aura: Option<(f64, f32, f32)>,
    pub aura_timer: f32,
    /// MinHealth (flee at or below it; 0 = never) and FearSustainTime.
    pub min_health: f32,
    pub fear_sustain: f32,
    pub flee_dest: Option<DVec3>,
    pub flee_repick: f32,
    /// Weapons dropped (ShouldDropWeapon: an arm gone), so the body doesn't hold them too.
    pub dropped: Vec<String>,
    /// CatchFire / UpdateFire: seconds burning (None = not on fire), the 1 s damage timer, and BurnPeriod.
    pub on_fire: Option<f32>,
    pub burn_tick: f32,
    pub burn_period: f32,
    /// Barks: the state (and seek) seen last tick, to bark on changes; bPlayIdle and the Standing idle clock.
    pub bark_prev: NpcState,
    pub bark_seek: Option<(SeekKind, bool)>,
    pub play_idle: bool,
    pub idle_bark: f32,
    /// ScriptedPawn.ShadowScale (blob shadow size).
    pub shadow_scale: f32,
}

fn fv(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}
/// Enhancement "NPC Ammo Runs Out" (or DXVIEW_NPC_FINITE_AMMO=1): pawns use up ammo, reload every clip and swap
/// weapons when dry, as DeusExWeapon's script does.
fn npc_finite_ammo() -> bool {
    static ENV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| std::env::var("DXVIEW_NPC_FINITE_AMMO").is_ok_and(|v| v == "1")) || crate::settings::get("NPCFiniteAmmo") > 0.5
}

fn flag(p: &[Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}
/// An actor's Tag (the class name when the map doesn't set one).
pub fn actor_tag(a: &dx_level::actors::ActorInfo) -> String {
    let t = name_of(&a.props, "Tag");
    if t.is_empty() {
        a.class.clone()
    } else {
        t
    }
}

fn name_of(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(s)) if s != "None" => s.clone(),
        // BindName and the like are strings.
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

impl Npc {
    /// ScriptedPawn.SetOrders(orderName, newOrderTag, bImmediate=true).
    pub fn set_orders(
        &mut self,
        lvl: &dx_level::Level,
        orders: &str,
        orders_tag: &str,
        player: DVec3,
        now: f32,
    ) {
        println!(
            "ORDERS {} ({}) -> {} {} state {:?}",
            self.bind_name, lvl.actors[self.actor].class, orders, orders_tag, self.state
        );
        let tagged = || {
            lvl.actors
                .iter()
                .filter(|a| name_of(&a.props, "Tag").eq_ignore_ascii_case(orders_tag))
                .map(|a| {
                    let l = a.location();
                    DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
                })
                .min_by(|a, b| a.distance(player).total_cmp(&b.distance(player)))
        };
        match orders.to_ascii_lowercase().as_str() {
            "attacking" | "fleeing" | "alerting" | "seeking" => {
                // Hostile orders: the order tag's pawn (the player when there's none) becomes the enemy.
                if orders_tag.is_empty() {
                    self.alliances.change_ally("Player", -1.0, true, false);
                    self.reaction = 1.0;
                    self.target = Some(Target::Player);
                    self.last_seen = now;
                    self.last_pos = player;
                    self.state = NpcState::Attacking;
                    self.weapon_drawn = true;
                }
            }
            o => {
                self.orders = match o {
                    "patrolling" => Orders::Patrolling,
                    "wandering" => Orders::Wandering,
                    "sitting" => Orders::Sitting,
                    "runningto" | "following" => Orders::RunningTo,
                    "goingto" => Orders::GoingTo,
                    "waitingfor" => Orders::WaitingFor,
                    _ => Orders::Standing,
                };
                self.order_tag = orders_tag.to_string();
                self.order_goal = if orders_tag.is_empty() {
                    None
                } else {
                    tagged()
                };
                self.route = None;
                self.patrol_dest = None;
                self.wander_goal = None;
                self.pause = 0.0;
                self.orders_failed = false;
                if self.orders == Orders::Standing && o != "standing" {
                    self.home = self.body.as_ref().map_or(self.home, |b| b.location);
                }
            }
        }
    }

    pub fn new(actor: usize, blocker: usize, p: &[Property], yaw_units: i32, home: DVec3) -> Npc {
        let initial: Vec<(String, f32, bool)> = p
            .iter()
            .filter(|x| x.name == "InitialAlliances")
            .filter_map(|x| match &x.value {
                Value::Alliance(n, l, perm) if !n.is_empty() && n != "None" => {
                    Some((n.clone(), *l, *perm))
                }
                _ => None,
            })
            .collect();
        let mut alliances =
            imsim_core::alliance::AllianceTable::new(&name_of(p, "Alliance"), initial);
        alliances.max_provocations = match find(p, "MaxProvocations") {
            Some(Value::Int(n)) => *n,
            _ => 1,
        };
        alliances.sustain_time = fv(p, "AgitationSustainTime", 30.0);
        alliances.decay_rate = fv(p, "AgitationDecayRate", 0.05);
        let rot_rate = match find(p, "RotationRate") {
            Some(Value::Rotator(r)) => r[1] as f64,
            _ => 20000.0,
        };
        let mut params = MoveParams::default();
        params.radius = fv(p, "CollisionRadius", 20.0) as f64;
        params.height = fv(p, "CollisionHeight", 47.5) as f64;
        params.stop_at_ledges = true;
        params.ground_speed = fv(p, "GroundSpeed", 200.0) as f64;
        params.accel_rate = fv(p, "AccelRate", 500.0) as f64;
        params.max_step_height = fv(p, "MaxStepHeight", 25.0) as f64;

        let orders = match name_of(p, "Orders").to_ascii_lowercase().as_str() {
            "patrolling" => Orders::Patrolling,
            "wandering" => Orders::Wandering,
            "sitting" => Orders::Sitting,
            _ => Orders::Standing,
        };
        let yaw = yaw_units as f64 / 65536.0 * std::f64::consts::TAU;
        Npc {
            actor,
            blocker,
            alliance: name_of(p, "Alliance"),
            alliances,
            state: NpcState::Idle,
            target: None,
            last_seen: -100.0,
            last_pos: DVec3::ZERO,
            yaw,
            body: None,
            params,
            weapon: None,
            inited: false,
            loaded: 0,
            shot_timer: 0.0,
            reload_timer: 0.0,
            sight_radius: fv(p, "SightRadius", 2500.0) as f64,
            tick_visible: !matches!(find(p, "bTickVisibleOnly"), Some(Value::Bool(false))),
            last_rendered: -1.0e9,
            render_check: 0.0,
            fov_cos: (fv(p, "AIHorizontalFov", 160.0) as f64 / 2.0)
                .to_radians()
                .cos(),
            base_accuracy: fv(p, "BaseAccuracy", 0.2),
            surprise: fv(p, "SurprisePeriod", 2.0),
            enemy_timeout: fv(p, "EnemyTimeout", 5.0),
            yaw_rate: rot_rate / 65536.0 * std::f64::consts::TAU,
            anim: "",
            think: (actor as f32 * 0.13) % 0.25,
            seed: 0x2545_F491_4F6C_DD1D ^ (actor as u64).wrapping_mul(0x9E37_79B9),
            orders,
            order_tag: name_of(p, "OrderTag"),
            order_goal: None,
            walk_frac: fv(p, "WalkingSpeed", 0.4) as f64,
            walk_anim_mult: fv(p, "walkAnimMult", 0.7),
            run_anim_mult: fv(p, "runAnimMult", 1.0),
            restlessness: fv(p, "Restlessness", 0.5),
            wanderlust: fv(p, "Wanderlust", 0.5),
            stand_height: fv(p, "CollisionHeight", 47.5) as f64,
            melee_wait: None,
            keep_drawn: matches!(find(p, "bKeepWeaponDrawn"), Some(Value::Bool(true))),
            weapon_drawn: false,
            home,
            home_yaw: yaw,
            use_home: true,
            home_extent: match find(p, "HomeExtent") {
                Some(Value::Float(v)) => *v as f64,
                _ => 800.0,
            },
            defend_home: matches!(find(p, "bDefendHome"), Some(Value::Bool(true))),
            animal: find(p, "FoodHealth").is_some()
                || matches!(name_of(p, "Alliance").as_str(), "Rat" | "Fly"),
            wander_goal: None,
            patrol_dest: None,
            pause: 0.0,
            pause_yaw: None,
            route: None,
            sitting: false,
            seat: None,
            orders_failed: false,
            hp_body: crate::damage::Body::new(p),
            take_hit_timer: 0.0,
            knocked_out: false,
            searched: false,
            just_fired: false,
            readiness: 0.0,
            reaction: 0.0,
            sustain: 0.0,
            cycle: 0.0,
            vis_threshold: fv(p, "VisibilityThreshold", 0.01),
            hear_threshold: fv(p, "HearingThreshold", 0.15),
            sight_pct: fv(p, "SightPercentage", 0.5),
            react_presence: flag(p, "bReactPresence"),
            react_loud: flag(p, "bReactLoudNoise"),
            react_shot: flag(p, "bReactShot"),
            react_distress: flag(p, "bReactDistress"),
            react_alarm: flag(p, "bReactAlarm"),
            react_carcass: flag(p, "bReactCarcass"),
            hate_carcass: flag(p, "bHateCarcass"),
            fear_carcass: flag(p, "bFearCarcass"),
            react_projectiles: flag(p, "bReactProjectiles"),
            react_futz: flag(p, "bReactFutz"),
            hate_shot: flag(p, "bHateShot"),
            fear_shot: flag(p, "bFearShot"),
            hate_weapon: flag(p, "bHateWeapon"),
            fear_weapon: flag(p, "bFearWeapon"),
            hate_hacking: flag(p, "bHateHacking"),
            fear_hacking: flag(p, "bFearHacking"),
            fear_alarm: flag(p, "bFearAlarm"),
            fear_distress: flag(p, "bFearDistress"),
            fear_injury: flag(p, "bFearInjury"),
            fear_indirect_injury: flag(p, "bFearIndirectInjury"),
            hate_indirect_injury: flag(p, "bHateIndirectInjury"),
            fear_projectiles: flag(p, "bFearProjectiles"),
            fear: imsim_core::reactions::Fear::new(fv(p, "FearSustainTime", 25.0), fv(p, "FearDecayRate", 0.5)),
            futz_timer: 0.0,
            fear_flee: false,
            cower: flag(p, "bCower"),
            stim_check: 0.0,
            avoid_harm: flag(p, "bAvoidHarm"),
            raise_alarm: match find(p, "RaiseAlarm") {
                Some(Value::Byte(b)) => *b,
                _ => 2,
            },
            alarm_timer: 0.0,
            alert_pending: false,
            body_alarm: None,
            alert: None,
            carcasses_seen: Vec::new(),
            carcass_check: 0.0,
            potential_enemy: None,
            killer_alliance: String::new(),
            killer_bind: String::new(),
            killswitch: None,
            emit_carcass: false,
            avoiding: 0.0,
            can_bleed: !matches!(find(p, "bCanBleed"), Some(Value::Bool(false))),
            service_cooldown: 0.0,
            service_pause: 0.0,
            flee_big: matches!(find(p, "bFleeBigPawns"), Some(Value::Bool(true)))
                || find(p, "majorDir").is_some(),
            cleaner: find(p, "majorDir").is_some(),
            clean_time: 0.0,
            stall: 0.0,
            can_open_doors: !matches!(find(p, "bCanOpenDoors"), Some(Value::Bool(false))),
            door: None,
            stalls: 0,
            minor_dir: match find(p, "minorDir") {
                Some(Value::Byte(b)) => *b,
                _ => 0,
            },
            major_dir: match find(p, "majorDir") {
                Some(Value::Byte(b)) => *b,
                _ => 0,
            },
            startle_timer: 0.0,
            squeak_timer: 0.0,
            service_shown: false,
            can_crouch: flag(p, "bCanCrouch"),
            can_strafe: flag(p, "bCanStrafe"),
            can_swim: flag(p, "bCanSwim"),
            must_face: flag(p, "bMustFaceTarget"),
            fire_angle: fv(p, "FireAngle", 360.0),
            atk_phase: AtkPhase::TurnFirst,
            crouch_rate: fv(p, "CrouchRate", 0.25),
            crouching: false,
            crouch_timer: 0.0,
            crouch_rolled: false,
            bind_name: name_of(p, "BindName"),
            important: flag(p, "bImportant"),
            bleed_rate: 0.0,
            drop_counter: 0.0,
            clot_period: fv(p, "ClotPeriod", 30.0),
            hate_distress: flag(p, "bHateDistress"),
            hate_injury: flag(p, "bHateInjury"),
            seek: None,
            seek_level: 0,
            ammo: None,
            poison_counter: 0,
            poison_timer: 0.0,
            poison_damage: 0.0,
            poisoner: None,
            bleed_counter: 0,
            bleed_timer: 0.0,
            bleed_damage: 0.0,
            carried: false,
            emp_hp: fv(p, "EMPHitPoints", 0.0),
            arsenal: Vec::new(),
            ammo_left: Vec::new(),
            weapon_timer: 10.0,
            switch_check: 0.0,
            fire_timer: 0.0,
            swap_carcass: None,
            crazed: 0.0,
            water_carcass: None,
            float_body: None,
            aura: None,
            aura_timer: 0.0,
            min_health: fv(p, "MinHealth", 0.0),
            fear_sustain: fv(p, "FearSustainTime", 25.0),
            flee_dest: None,
            flee_repick: 0.0,
            dropped: Vec::new(),
            on_fire: None,
            burn_tick: 0.0,
            burn_period: fv(p, "BurnPeriod", 30.0),
            bark_prev: NpcState::Idle,
            bark_seek: None,
            play_idle: flag(p, "bPlayIdle"),
            shadow_scale: fv(p, "ShadowScale", 1.0),
            idle_bark: 8.0 + crate::sound::frand() * 14.0,
        }
    }

    /// ScriptedPawn's Play<Mode>Sound calls (StartAIBarkConversation), from the state changes they sit on.
    fn barks(&mut self, dt: f32, standing_still: bool) {
        use crate::consys::BarkMode as B;
        let prev = self.bark_prev;
        let now = self.state;
        let seek = self.seek.as_ref().map(|s| (s.kind, s.post_combat));
        let mut out = None;
        if std::mem::discriminant(&prev) != std::mem::discriminant(&now) {
            out = match now {
                // Surprise: only when the pause is long enough to play out ((1-ReactionLevel) x SurprisePeriod >= 0.25).
                NpcState::Surprised(w) if w >= 0.25 => Some(B::Surprise),
                // BeginAttack: PlayTargetAcquiredSound.
                NpcState::Attacking => Some(B::TargetAcquired),
                // Fleeing in fright: the Surprise label's PlaySurpriseSound; hurt or dry: CriticalDamage / OutOfAmmo.
                NpcState::Fleeing if self.fear_flee => Some(B::Surprise),
                NpcState::Fleeing => Some(if self.should_flee() { B::CriticalDamage } else { B::OutOfAmmo }),
                NpcState::Alerting => Some(B::GoingForAlarm),
                NpcState::Burning => Some(B::OnFire),
                NpcState::RubbingEyes(_) if !matches!(prev, NpcState::RubbingEyes(_)) => Some(B::TearGas),
                // Done fighting: the enemy's gone (FindBestEnemy -> AreaSecure), or the search gave up.
                NpcState::Idle => match prev {
                    NpcState::Attacking => Some(B::AreaSecure),
                    NpcState::Seeking => Some(match self.bark_seek {
                        Some((_, true)) => B::TargetLost,
                        _ => B::SearchGiveUp,
                    }),
                    _ => None,
                },
                _ => None,
            };
        }
        // Seeking.Begin: what started the search.
        if now == NpcState::Seeking && seek.is_some() && (prev != NpcState::Seeking || seek.map(|s| s.0) != self.bark_seek.map(|s| s.0)) {
            out = seek.and_then(|(k, post)| {
                if post {
                    Some(B::PostAttackSearching)
                } else {
                    match k {
                        SeekKind::Sound => Some(B::PreAttackSearching),
                        SeekKind::Sight => Some(B::PreAttackSighting),
                        SeekKind::Carcass => Some(B::Gore),
                        SeekKind::Guess => Some(B::PostAttackSearching),
                    }
                }
            });
        }
        // Standing: Sleep(8..22 s) and a fidget, then a 30% chance of PlayIdleSound.
        if now == NpcState::Idle && standing_still && self.play_idle {
            self.idle_bark -= dt;
            if self.idle_bark <= 0.0 {
                self.idle_bark = 8.0 + crate::sound::frand() * 14.0 + 4.0;
                if crate::sound::frand() < 0.3 {
                    out = out.or(Some(B::Idle));
                }
            }
        }
        self.bark_prev = now;
        if seek.is_some() {
            self.bark_seek = seek;
        }
        if let Some(m) = out {
            crate::consys::bark(self.blocker, m);
        }
    }

    /// GetPawnAllianceType == ALLIANCE_Friendly.
    fn friendly(&self, alliance: &str) -> bool {
        !alliance.is_empty()
            && (alliance.eq_ignore_ascii_case(&self.alliance)
                || self.alliances.friendly_to(alliance))
    }

    /// SetSeekLocation: forget the enemy and remember where to look.
    fn set_seek_location(&mut self, pos: DVec3, kind: SeekKind, post_combat: bool) {
        self.target = None;
        let s = self.seek.get_or_insert(Seek {
            kind,
            pos,
            dest: pos,
            located: true,
            phase: SeekPhase::Begin,
            post_combat: false,
        });
        s.kind = kind;
        s.pos = pos;
        s.located = true;
        if post_combat {
            s.post_combat = true;
        }
        self.seek_level = self
            .seek_level
            .max(if kind == SeekKind::Sight { 1 } else { 3 });
    }

    /// GotoState('Seeking'): stand up, draw the weapon, start at Begin.
    fn enter_seeking(&mut self) {
        self.stand_up();
        self.state = NpcState::Seeking;
        self.weapon_drawn = true;
        self.route = None;
        self.pause = 0.0;
        if let Some(s) = self.seek.as_mut() {
            s.phase = SeekPhase::Begin;
        }
    }

    /// A new place to look while already seeking (Seeking.HandleSighting / HandleLoudNoise): interrupts
    /// GoToLocation/TurnToLocation; during the wait it cuts the look-around short.
    fn reseek(&mut self, pos: DVec3, kind: SeekKind) {
        self.set_seek_location(pos, kind, false);
        if let Some(s) = self.seek.as_mut() {
            s.dest = pos;
            if matches!(
                s.phase,
                SeekPhase::Go | SeekPhase::Overshoot(_) | SeekPhase::Turn
            ) {
                s.phase = SeekPhase::Go;
                self.route = None;
            }
        }
    }

    /// Leave Seeking and go back to orders (DoneSeek -> FollowOrders).
    fn finish_seek(&mut self) {
        self.state = NpcState::Idle;
        self.seek = None;
        self.seek_level = 0;
        self.route = None;
        self.patrol_dest = None;
        self.orders_failed = false;
    }

    /// SetEnemy + HandleEnemy: attack, after the surprise pause that ReactionLevel hasn't covered yet.
    fn begin_attack(&mut self, t: Target, pos: DVec3, seen_at: f32) {
        self.target = Some(t);
        self.fear_flee = false;
        self.last_pos = pos;
        self.last_seen = seen_at;
        self.readiness = 1.0;
        if matches!(self.state, NpcState::Idle | NpcState::Seeking) {
            self.stand_up();
            self.seek = None;
            self.seek_level = 0;
            self.route = None;
            self.pause = 0.0;
            let wait = (1.0 - self.reaction) * self.surprise;
            self.state = if wait < 0.25 {
                NpcState::Attacking
            } else {
                NpcState::Surprised(wait)
            };
            // HandlingEnemy: RAISEALARM_BeforeAttacking runs for an alarm first (if there is one to push).
            self.alert_pending = self.raise_alarm == 0 && self.alarm_timer <= 0.0;
        }
    }

    fn stand_up(&mut self) {
        if self.sitting {
            self.sitting = false;
            self.seat = None;
            if let Some(b) = self.body.as_mut() {
                b.physics = Physics::Falling;
            }
        }
    }

    /// GetAllianceType == ALLIANCE_Hostile.
    /// GetAllianceType with bReverseAlliances (a scrambled robot): friends are enemies and enemies friends.
    pub fn hates(&self, alliance: &str) -> bool {
        if self.crazed > 0.0 {
            self.alliances.friendly_to(alliance)
        } else {
            self.alliances.hostile_to(alliance)
        }
    }

    /// The ammo class a weapon would load: what the pawn was given if the weapon takes it, else its AmmoName.
    fn ammo_class_for(&self, w: &WeaponStats) -> Option<String> {
        if let Some(a) = &self.ammo {
            if w.ammo_names.iter().any(|n| n.eq_ignore_ascii_case(a))
                || w.ammo_class
                    .as_deref()
                    .map_or(false, |c| c.eq_ignore_ascii_case(a))
            {
                return Some(a.clone());
            }
        }
        w.ammo_class.clone()
    }

    fn ammo_count(&self, class: &str) -> i32 {
        self.ammo_left
            .iter()
            .find(|x| x.0.eq_ignore_ascii_case(class))
            .map(|x| x.1)
            .unwrap_or(0)
    }

    /// Rounds left for the weapon in hand (None for weapons that need none).
    fn weapon_ammo(&self) -> Option<i32> {
        let w = self.weapon.as_ref()?;
        if w.reload_count == 0 {
            return None;
        }
        Some(
            self.ammo_class_for(w)
                .map(|a| self.ammo_count(&a))
                .unwrap_or(0),
        )
    }

    fn use_ammo(&mut self) {
        let Some(w) = self.weapon.as_ref() else {
            return;
        };
        if w.reload_count == 0 {
            return;
        }
        if let Some(a) = self.ammo_class_for(w) {
            if let Some(e) = self
                .ammo_left
                .iter_mut()
                .find(|x| x.0.eq_ignore_ascii_case(&a))
            {
                e.1 = (e.1 - 1).max(0);
            }
        }
    }

    /// ScriptedPawn.SwitchToBestWeapon: score every usable weapon by how far the enemy is from the middle of its
    /// best range (lower is better), prefer non-fallback weapons (knives) while there's ammo, and stick with a new
    /// choice for a while. `enemy` = (distance, radius, is_robot, is_player).
    fn switch_to_best_weapon(&mut self, enemy: Option<(f64, f64, bool, bool)>) {
        let (enemy_range, enemy_radius, robot, player) =
            enemy.unwrap_or((400.0, 0.0, false, false));
        let mut best: Option<(usize, f64, i32)> = None;
        for i in 0..self.arsenal.len() {
            let w = self.arsenal[i].clone();
            let mut valid = true;
            if w.reload_count > 0 {
                valid = self
                    .ammo_class_for(&w)
                    .map(|a| self.ammo_count(&a) >= 1)
                    .unwrap_or(false);
            }
            if !valid {
                continue;
            }
            // GetWeaponBestRange (projectile weapons: no minimum for our purposes; AIMin/MaxRange if set).
            let (wmin, wacc) = (0.0f64, w.accurate_range as f64);
            let min_r = if w.ai_min_range > 0.0 && (w.ai_min_range as f64) <= wacc {
                w.ai_min_range as f64
            } else {
                wmin
            };
            let acc_r = if w.ai_max_range > 0.0
                && (w.ai_max_range as f64) >= min_r
                && (w.ai_max_range as f64) <= wacc
            {
                w.ai_max_range as f64
            } else {
                wacc
            };
            let min_r = min_r.min(acc_r);
            let cutoff = min_r + self.params.radius + enemy_radius;
            let range = ((acc_r - min_r) * 0.5).max(50.0);
            let center = min_r + (acc_r - min_r) * 0.5;
            let mut score = (enemy_range - center).abs() / range;
            if cutoff >= enemy_range - self.params.radius && cutoff >= 256.0 {
                score += 10000.0;
            }
            let mut level = 3;
            if w.fallback {
                level = 2;
            }
            if w.ai_fire_delay > 0.0 && self.fire_timer > 0.0 {
                level = 0;
            }
            if w.stun && player {
                score += 10.0;
            }
            if robot
                && w.projectile
                    .as_deref()
                    .map_or(false, |p| p.contains("Poison"))
            {
                continue;
            }
            let current = self.weapon.as_ref().map_or(false, |c| c.class == w.class);
            if current && self.weapon_timer < 10.0 {
                if self.weapon_timer < 5.0 {
                    score = -10.0;
                } else {
                    score -= ((10.0 - self.weapon_timer) / 5.0) as f64;
                }
            } else {
                score = (score + self.rand() as f64 * 0.1 - 0.05).max(0.0);
            }
            let win = match best {
                None => true,
                Some((_, bs, bl)) => level > bl || (level == bl && bs > score),
            };
            if win {
                best = Some((i, score, level));
            }
        }
        let new = best.map(|b| self.arsenal[b.0].clone());
        let changed =
            new.as_ref().map(|w| w.class.clone()) != self.weapon.as_ref().map(|w| w.class.clone());
        if changed {
            self.weapon_timer = if enemy.is_none() { 10.0 } else { 0.0 };
            self.reload_timer = 0.0;
            self.weapon = new;
            if let Some(w) = self.weapon.clone() {
                let left = self.weapon_ammo().unwrap_or(i32::MAX);
                self.loaded = (w.reload_count.max(1) as i32).min(left).max(0) as u32;
                if let Some(a) = self.ammo_class_for(&w) {
                    if w.reload_count > 0 {
                        self.ammo = Some(a);
                    }
                }
            }
        }
    }

    /// ScriptedPawn.ShouldFlee: at or below MinHealth, or any limb gone (only for pawns with a MinHealth).
    fn should_flee(&self) -> bool {
        let b = &self.hp_body;
        self.min_health > 0.0
            && (b.health <= self.min_health || b.parts[2..6].iter().any(|&h| h <= 0.0))
    }

    /// ShouldDropWeapon: an arm is gone.
    fn should_drop_weapon(&self) -> bool {
        let b = &self.hp_body;
        (b.parts[2] <= 0.0 || b.parts[3] <= 0.0) && b.health > 0.0
    }

    /// SetReactions for the state this pawn is in (ScriptedPawn BeginState / EndState calls): the orders states
    /// (Standing, Patrolling, Wandering, Sitting, ...) react to everything, Seeking ignores alarms and bodies,
    /// AvoidingProjectiles ignores projectiles, a conversation blocks everything, and the busy states (Attacking,
    /// Fleeing, Alerting, Burning, TakingHit, Stunned, RubbingEyes, the surprise before HandlingEnemy) only notice
    /// being hurt.
    pub fn looking_for(&self, conversing: bool, first_person: bool) -> imsim_core::reactions::LookingFor {
        use imsim_core::reactions::LookingFor as L;
        if conversing {
            return if first_person { L::FIRST_PERSON_CONVERSATION } else { L::blocked(true) };
        }
        match self.state {
            NpcState::Idle if self.avoiding > 0.0 => L::AVOIDING_PROJECTILES,
            NpcState::Idle => L::ALL,
            NpcState::Seeking => L::SEEKING,
            NpcState::Dead => L::blocked(true),
            _ => L::blocked(false),
        }
    }

    /// SetEnemy(pawn, , true) + GotoState('Fleeing'): run from `t`; `fear` when it's fright rather than harm (the
    /// flight then ends once no longer fearful, hostile or not).
    fn flee_from(&mut self, t: Target, pos: DVec3, now: f32, fear: bool) {
        self.stand_up();
        self.target = Some(t);
        self.last_pos = pos;
        self.last_seen = now;
        self.fear_flee = fear;
        if self.state != NpcState::Fleeing {
            self.enter_fleeing();
        }
    }

    /// ReactToFutz: a futz bark, at most every 2 s, not while distressed.
    fn react_to_futz(&mut self) {
        if self.react_futz && self.futz_timer <= 0.0 && !matches!(self.state, NpcState::Fleeing | NpcState::Attacking) {
            self.futz_timer = 2.0;
            crate::consys::bark(self.blocker, crate::consys::BarkMode::Futz);
        }
    }

    fn enter_fleeing(&mut self) {
        self.state = NpcState::Fleeing;
        self.route = None;
        self.flee_dest = None;
        self.flee_repick = 0.0;
        self.weapon_drawn = false;
        self.seek = None;
    }

    fn two_handed(&self) -> bool {
        self.weapon_drawn
            && self
                .weapon
                .as_ref()
                .map(|w| w.mass >= 30.0)
                .unwrap_or(false)
    }

    fn rand(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.seed >> 33) as f32) / (1u64 << 31) as f32
    }
}

/// Give every pawn an `Npc` brain (run once after spawning).
pub fn attach_npcs(
    mut commands: Commands,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    q: Query<(Entity, &Deco), Without<Npc>>,
    mut map_gen: Local<u64>,
    mut done: Local<bool>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *done = false;
    }
    if *done {
        return;
    }
    *done = true;
    for (e, d) in &q {
        // Bodies placed in the map (TerroristCarcass etc.) are already dead pawns with InitialInventory to search.
        let ma = &level.0.mesh_actors[d.mesh_actor];
        let ca = &level.0.actors[ma.actor];
        if ca.class.ends_with("Carcass") {
            let mut n = Npc::new(
                ma.actor,
                d.blocker.unwrap_or(usize::MAX),
                &ca.props,
                d.rotation[1],
                d.body.location,
            );
            n.state = NpcState::Dead;
            n.inited = true;
            commands.entity(e).insert(n);
            continue;
        }
        let Some(b) = d.blocker else { continue };
        let ai = blockers.list[b].actor;
        let a = &level.0.actors[ai];
        if find(&a.props, "GroundSpeed").is_none() {
            continue;
        }
        commands
            .entity(e)
            .insert(Npc::new(ai, b, &a.props, d.rotation[1], d.body.location));
        // bInWorld=False: placed but out of the world until a script calls EnterWorld.
        if matches!(find(&a.props, "bInWorld"), Some(Value::Bool(false))) {
            let c = &mut blockers.list[b];
            commands.entity(e).insert((
                OutOfWorld {
                    radius: fv(&a.props, "CollisionRadius", c.radius as f32) as f64,
                    height: c.height,
                },
                Visibility::Hidden,
            ));
            c.radius = 0.0;
        }
    }
}

/// A pawn that has left the world (ScriptedPawn.LeaveWorld): hidden, no collision, no AI.
#[derive(Component)]
pub struct OutOfWorld {
    pub radius: f64,
    pub height: f64,
}

/// Which pawns a mission script means.
#[derive(Debug, Clone)]
pub enum Sel {
    Bind(String),
    Class(String),
    /// Of a class, except one BindName.
    ClassBut(String, String),
    /// Of a class with this Tag (foreach AllActors(class, P, tag)).
    ClassTag(String, String),
    /// Mission scripts: class by IsA (subclasses too), Tag, BindName - each optional.
    Pick { class: Option<String>, tag: Option<String>, bind: Option<String> },
}

impl Sel {
    pub fn isa(class: &str) -> Sel {
        Sel::Pick { class: Some(class.into()), tag: None, bind: None }
    }
    pub fn isa_tag(class: &str, tag: &str) -> Sel {
        Sel::Pick { class: Some(class.into()), tag: Some(tag.into()), bind: None }
    }
    pub fn tag(tag: &str) -> Sel {
        Sel::Pick { class: None, tag: Some(tag.into()), bind: None }
    }
    pub fn isa_bind(class: &str, bind: &str) -> Sel {
        Sel::Pick { class: Some(class.into()), tag: None, bind: Some(bind.into()) }
    }
}

/// Script requests on pawns (MissionScript: EnterWorld, Destroy, a fatal TakeDamage, SetLocation).
#[derive(Debug, Clone)]
pub enum WorldReq {
    Enter(Sel),
    Destroy(Sel),
    Kill(Sel),
    /// Location, yaw (Unreal units): SetLocation, SetRotation, SetOrders('Standing'), SetHomeBase.
    Teleport(Sel, DVec3, i32),
    /// ScriptedPawn.LeaveWorld: hidden, no collision, no AI until EnterWorld.
    Leave(Sel),
    /// Mission00: HealthTorso = 0, bStunned, TakeDamage(1, 'KnockedOut') - unconscious on the floor.
    KnockOut(Sel),
    /// bInvincible on / off.
    Invincible(Sel, bool),
    /// MinHealth (when it flees).
    MinHealth(Sel, f32),
    /// SetHomeBase(Location, Rotation): home where it stands now.
    HomeHere(Sel),
    /// SetHomeBase(point): home at a spot (Unreal location, yaw).
    HomeAt(Sel, DVec3, i32),
    /// HealthTorso = 0, Health = 0, TakeDamage(1, self, 'Shot'): dies with what it carries.
    KillShot(Sel),
    /// GuntherHermann state KillswitchActivated: convulses, then explodes.
    Killswitch(Sel),
    /// DesiredRotation = Rotator(spot - Location).
    Face(Sel, DVec3),
    /// Mission06 TeleportPawn: SetLocation + SetRotation + SetOrders(orders).
    TeleportOrders(Sel, DVec3, i32, String),
}

static WORLD: std::sync::Mutex<Vec<WorldReq>> = std::sync::Mutex::new(Vec::new());
/// Seats already taken by sitting pawns (Sitting.SitPosition).
static TAKEN: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

pub fn world_req(r: WorldReq) {
    if let Ok(mut q) = WORLD.lock() {
        q.push(r);
    }
}

fn sel_matches(sel: &Sel, class: &str, bind: &str) -> bool {
    sel_matches_tag(sel, class, bind, "")
}

fn sel_matches_tag(sel: &Sel, class: &str, bind: &str, tag: &str) -> bool {
    match sel {
        Sel::ClassTag(c, t) => class.eq_ignore_ascii_case(c) && tag.eq_ignore_ascii_case(t),
        Sel::Bind(b) => bind.eq_ignore_ascii_case(b),
        Sel::Class(c) => class.eq_ignore_ascii_case(c),
        Sel::ClassBut(c, b) => class.eq_ignore_ascii_case(c) && !bind.eq_ignore_ascii_case(b),
        Sel::Pick { class: c, tag: t, bind: b } => {
            c.as_ref().is_none_or(|c| crate::classtree::is_a(class, c))
                && t.as_ref().is_none_or(|t| t.eq_ignore_ascii_case(tag))
                && b.as_ref().is_none_or(|b| b.eq_ignore_ascii_case(bind))
        }
    }
}

/// For the PawnOrder pass (ai::npc_system): does this pawn match a selection?
pub fn sel_hit(sel: &Sel, class: &str, bind: &str, tag: &str) -> bool {
    sel_matches_tag(sel, class, bind, tag)
}

#[allow(clippy::type_complexity)]
pub fn world_system(
    mut commands: Commands,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mut hits: ResMut<crate::damage::PawnHits>,
    mut q: Query<(
        Entity,
        &mut Npc,
        &mut Deco,
        &mut Visibility,
        Option<&OutOfWorld>,
    )>,
) {
    let _prof = crate::prof::scope("ai::world_system");
    let reqs: Vec<WorldReq> = WORLD
        .lock()
        .map(|mut w| std::mem::take(&mut *w))
        .unwrap_or_default();
    if reqs.is_empty() {
        return;
    }
    for r in reqs {
        for (e, mut npc, mut deco, mut vis, out) in &mut q {
            let class = level.0.actors[npc.actor].class.as_str();
            let tag = name_of(&level.0.actors[npc.actor].props, "Tag");
            let dead = npc.state == NpcState::Dead;
            match &r {
                WorldReq::Leave(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && out.is_none() => {
                    if let Some(c) = blockers.list.get_mut(npc.blocker) {
                        commands.entity(e).insert((
                            OutOfWorld {
                                radius: c.radius,
                                height: c.height,
                            },
                        ));
                        c.radius = 0.0;
                    }
                    *vis = Visibility::Hidden;
                    crate::scriptworld::mark_hidden(npc.actor, true);
                    println!("LEAVEWORLD {class} {tag}");
                }
                WorldReq::KnockOut(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    hits.0.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: 10000.0,
                        damage_type: "KnockedOut",
                        hit_loc: deco.body.location,
                        attacker: Some(npc.blocker),
                        attacker_pos: deco.body.location,
                    });
                }
                WorldReq::Enter(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) => {
                    if let Some(o) = out {
                        if let Some(c) = blockers.list.get_mut(npc.blocker) {
                            c.radius = o.radius;
                            c.height = o.height;
                            c.center = deco.body.location;
                        }
                        commands.entity(e).remove::<OutOfWorld>();
                        *vis = Visibility::Inherited;
                        crate::scriptworld::mark_hidden(npc.actor, false);
                        println!("ENTERWORLD {}", npc.bind_name);
                    }
                }
                WorldReq::Destroy(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    if let Some(c) = blockers.list.get_mut(npc.blocker) {
                        c.radius = 0.0;
                    }
                    npc.state = NpcState::Dead;
                    commands.entity(e).despawn();
                    println!("DESTROY {} {}", class, npc.bind_name);
                }
                WorldReq::Kill(sel)
                    if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead && out.is_none() =>
                {
                    // Mission01: HealthTorso = 0, TakeDamage(1, self, 'Shot'), inventory deleted.
                    npc.arsenal.clear();
                    npc.weapon = None;
                    npc.ammo_left.clear();
                    hits.0.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: 10000.0,
                        damage_type: "Shot",
                        hit_loc: deco.body.location,
                        attacker: Some(npc.blocker),
                        attacker_pos: deco.body.location,
                    });
                }
                WorldReq::Teleport(sel, pos, yaw) | WorldReq::TeleportOrders(sel, pos, yaw, _)
                    if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead =>
                {
                    // SetLocation -> FarMoveActor: a spot whose cylinder dips into the world is nudged out
                    // (FindSpot) - SpawnPoints sit low (PaulTeleport is 15 units off the dock).
                    let ext = DVec3::new(npc.params.radius, npc.params.radius, npc.params.height);
                    let top = *pos + DVec3::new(0.0, 0.0, npc.params.height);
                    let h = level.0.collision.trace(top, *pos, ext);
                    let pos = &(if h.fraction < 1.0 { top + (*pos - top) * h.fraction } else { *pos });
                    println!("TELEPORT {} -> {:.0},{:.0},{:.0}", npc.bind_name, pos.x, pos.y, pos.z);
                    deco.body.location = *pos;
                    deco.body.velocity = DVec3::ZERO;
                    if let Some(b) = npc.body.as_mut() {
                        b.location = *pos;
                        b.velocity = DVec3::ZERO;
                    }
                    let y = *yaw as f64 / 65536.0 * std::f64::consts::TAU;
                    npc.yaw = y;
                    npc.home = *pos;
                    npc.home_yaw = y;
                    npc.use_home = true;
                    npc.orders = Orders::Standing;
                    npc.route = None;
                    npc.orders_failed = false;
                    if let Some(c) = blockers.list.get_mut(npc.blocker) {
                        c.center = *pos;
                    }
                    if let WorldReq::TeleportOrders(sel, _, _, o) = &r {
                        if !o.eq_ignore_ascii_case("Standing") {
                            crate::triggers::push_order(crate::triggers::PawnOrder::Script {
                                sel: sel.clone(),
                                orders: o.clone(),
                                orders_tag: String::new(),
                            });
                        }
                    }
                }
                WorldReq::Invincible(sel, on) if sel_matches_tag(sel, class, &npc.bind_name, &tag) => {
                    npc.hp_body.invincible = *on;
                }
                WorldReq::MinHealth(sel, v) if sel_matches_tag(sel, class, &npc.bind_name, &tag) => {
                    npc.min_health = *v;
                }
                WorldReq::HomeHere(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    npc.home = deco.body.location;
                    npc.home_yaw = npc.yaw;
                    npc.use_home = true;
                }
                WorldReq::HomeAt(sel, pos, yaw) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    npc.home = *pos;
                    npc.home_yaw = *yaw as f64 / 65536.0 * std::f64::consts::TAU;
                    npc.use_home = true;
                }
                WorldReq::KillShot(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    npc.hp_body.invincible = false;
                    hits.0.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: 10000.0,
                        damage_type: "Shot",
                        hit_loc: deco.body.location,
                        attacker: Some(npc.blocker),
                        attacker_pos: deco.body.location,
                    });
                }
                WorldReq::Killswitch(sel) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    npc.hp_body.invincible = false;
                    npc.killswitch = Some(0.0);
                    npc.target = None;
                    npc.route = None;
                    println!("KILLSWITCH {}", npc.bind_name);
                }
                WorldReq::Face(sel, at) if sel_matches_tag(sel, class, &npc.bind_name, &tag) && !dead => {
                    let d = *at - deco.body.location;
                    npc.yaw = d.y.atan2(d.x);
                    npc.pause_yaw = Some(npc.yaw);
                }
                _ => {}
            }
        }
    }
}

fn target_pos(t: Target, player_center: DVec3, list: &[Cylinder]) -> DVec3 {
    match t {
        Target::Player => player_center,
        Target::Pawn(b) => list[b].center,
    }
}

fn eye_of(c: &Cylinder) -> DVec3 {
    c.center + DVec3::new(0.0, 0.0, c.height * 0.8)
}

fn turn_toward(yaw: f64, want: f64, max_step: f64) -> f64 {
    let mut d = (want - yaw) % std::f64::consts::TAU;
    if d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    if d < -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    yaw + d.clamp(-max_step, max_step)
}

/// Play `name` (or `fallback` if the mesh lacks it) looping at `mult` times its rate.
fn set_anim(
    an: &mut Animated,
    level: &LevelRes,
    npc: &mut Npc,
    name: &'static str,
    fallback: &'static str,
    mult: f32,
) {
    let ma = &level.0.mesh_actors[an.mesh_actor];
    let Some(anim) = &ma.anim else { return };
    let Some(n) = [name, fallback].into_iter().find(|n| {
        !n.is_empty()
            && anim
                .mesh
                .seqs
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(n))
    }) else {
        return;
    };
    if npc.anim == n {
        return;
    }
    if let Some(mut c) = Clip::named(&anim.mesh, n, true) {
        c.rate *= mult;
        an.clip = Some(c);
        npc.anim = n;
    }
}

fn flat_dist(a: DVec3, b: DVec3) -> f64 {
    DVec3::new(a.x - b.x, a.y - b.y, 0.0).length()
}

/// Clear walking line between two points for a pawn of this size (world geometry only).
fn clear_line(level: &dx_level::Level, from: DVec3, to: DVec3, radius: f64) -> bool {
    let e = DVec3::new(radius * 0.6, radius * 0.6, 8.0);
    let up = DVec3::new(0.0, 0.0, 12.0);
    if level.collision.trace(from + up, to + up, e).fraction < 0.999 {
        return false;
    }
    // And floor all the way (no walking off a dock into the water): every 40 units there must be ground within
    // 100 units below the line (centres are a collision height above the floor).
    let len = (to - from).truncate().length();
    let n = (len / 40.0) as i32;
    for k in 1..n {
        let q = from + (to - from) * (k as f64 / n as f64);
        if level
            .collision
            .trace(q, q - DVec3::new(0.0, 0.0, 100.0), DVec3::ZERO)
            .fraction
            >= 0.999
        {
            return false;
        }
    }
    true
}

/// Plan a route to `goal`: straight there if the way is clear, else over the path network
/// (nearest visible nav point at each end, A* in between).
fn plan_route(
    level: &dx_level::Level,
    from: DVec3,
    goal: DVec3,
    p: &MoveParams,
    goal_node: Option<usize>,
) -> Route {
    plan_route_ex(level, from, goal, p, goal_node, false)
}

fn plan_route_ex(
    level: &dx_level::Level,
    from: DVec3,
    goal: DVec3,
    p: &MoveParams,
    goal_node: Option<usize>,
    force: bool,
) -> Route {
    let mut r = Route {
        goal,
        nodes: Vec::new(),
        age: 0.0,
        check_pos: from,
        check_time: 0.0,
        stuck: 0,
        forced: force,
        needs_graph: false,
    };
    if !force && goal_node.is_none() && clear_line(level, from, goal, p.radius) {
        return r;
    }
    let g = &level.nav;
    let a = |v: DVec3| [v.x, v.y, v.z];
    let start = g.nearby(a(from), 1500.0).into_iter().take(12).find(|&i| {
        let q = g.points[i].pos;
        clear_line(level, from, DVec3::new(q[0], q[1], q[2]), p.radius)
    });
    let end = goal_node.or_else(|| {
        g.nearby(a(goal), 1500.0).into_iter().take(12).find(|&i| {
            let q = g.points[i].pos;
            clear_line(level, DVec3::new(q[0], q[1], q[2]), goal, p.radius)
        })
    });
    if let (Some(s), Some(e)) = (start, end) {
        if let Some(path) = g.find_path(s, e, p.radius, p.height) {
            r.nodes = path;
        }
    }
    r
}

/// Follow `route`, re-planning over the path network if the straight line was blocked.
fn follow(level: &dx_level::Level, route: &mut Route, me: DVec3, p: &MoveParams, dt: f32) -> DVec3 {
    let w = follow_route(level, route, me, p.radius, dt);
    if route.needs_graph {
        let stuck = route.stuck;
        let goal = route.goal;
        let mut nr = plan_route_ex(level, me, goal, p, None, true);
        nr.stuck = stuck;
        *route = nr;
    }
    w
}

/// Steer along the current route; returns the direction to walk (zero when arrived).
fn follow_route(level: &dx_level::Level, r: &mut Route, me: DVec3, radius: f64, dt: f32) -> DVec3 {
    r.age += dt;
    r.check_time += dt;
    // Skip nodes we're at, or that are already behind a clear line to the next one.
    while let Some(&n) = r.nodes.first() {
        let q = level.nav.points[n].pos;
        let q = DVec3::new(q[0], q[1], q[2]);
        let next = r.nodes.get(1).map(|&m| {
            let w = level.nav.points[m].pos;
            DVec3::new(w[0], w[1], w[2])
        });
        let at = flat_dist(me, q) < radius.max(16.0) + 8.0 && (me.z - q.z).abs() < 80.0;
        let shortcut = match next {
            Some(w) => flat_dist(me, q) < 160.0 && clear_line(level, me, w, radius),
            None => flat_dist(me, q) < 160.0 && clear_line(level, me, r.goal, radius),
        };
        if at || shortcut {
            r.nodes.remove(0);
        } else {
            break;
        }
    }
    let aim = match r.nodes.first() {
        Some(&n) => {
            let q = level.nav.points[n].pos;
            DVec3::new(q[0], q[1], q[2])
        }
        None => r.goal,
    };
    // Progress watchdog: if we haven't moved in a while, drop the next node (or give up).
    if r.check_time > 1.5 {
        if me.distance(r.check_pos) < 16.0 {
            r.stuck += 1;
            if !r.nodes.is_empty() {
                r.nodes.remove(0);
            } else if !r.forced {
                r.needs_graph = true;
            }
        }
        r.check_pos = me;
        r.check_time = 0.0;
    }
    let d = DVec3::new(aim.x - me.x, aim.y - me.y, 0.0);
    if r.nodes.is_empty() && d.length() < 12.0 {
        return DVec3::ZERO;
    }
    d.normalize_or_zero()
}

/// ScriptedPawn.IsNearHome: within HomeExtent of home and with a clear line to it.
fn is_near_home(level: &dx_level::Level, npc: &Npc, pos: DVec3) -> bool {
    !npc.use_home
        || (npc.home.distance(pos) <= npc.home_extent
            && level.collision.trace(pos, npc.home, DVec3::ZERO).fraction >= 0.999)
}

/// Ordinary behavior: patrol, wander, stand or sit. Returns (wish direction, walking).
fn do_orders(
    level: &dx_level::Level,
    npc: &mut Npc,
    me: DVec3,
    dt: f32,
    player: DVec3,
    sees_player: bool,
) -> (DVec3, bool) {
    let run = matches!(npc.orders, Orders::RunningTo | Orders::WaitingFor);
    (
        do_orders_inner(level, npc, me, dt, player, sees_player),
        run,
    )
}

fn do_orders_inner(
    level: &dx_level::Level,
    npc: &mut Npc,
    me: DVec3,
    dt: f32,
    player: DVec3,
    sees_player: bool,
) -> DVec3 {
    let p = npc.params.clone();
    if npc.pause > 0.0 {
        npc.pause -= dt;
        if let Some(y) = npc.pause_yaw {
            npc.yaw = turn_toward(npc.yaw, y, npc.yaw_rate * dt as f64);
        }
        return DVec3::ZERO;
    }
    match npc.orders {
        Orders::Sitting => DVec3::ZERO,
        Orders::WaitingFor if !sees_player => {
            let to = player - me;
            npc.yaw = turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64 * 0.25);
            DVec3::ZERO
        }
        Orders::RunningTo | Orders::GoingTo | Orders::WaitingFor => {
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                static LAST: AtomicU64 = AtomicU64::new(0);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                if now_ms - LAST.load(Ordering::Relaxed) > 2000 {
                    LAST.store(now_ms, Ordering::Relaxed);
                    println!(
                        "ORDERS {:?} {} at {:.0},{:.0},{:.0} -> {:?} stuck={:?}",
                        npc.orders,
                        level.actors[npc.actor].class,
                        me.x,
                        me.y,
                        me.z,
                        npc.order_goal.unwrap_or(player),
                        npc.route.as_ref().map(|r| (r.nodes.len(), r.stuck))
                    );
                }
            }
            // Follow: until touching the order actor (IsOverlapping), then Standing where we are.
            let goal = npc.order_goal.unwrap_or(player);
            let reach = if npc.order_goal.is_none() {
                p.radius + 24.0 + 8.0
            } else {
                p.radius + 8.0
            };
            if flat_dist(me, goal) <= reach {
                npc.orders = Orders::Standing;
                npc.route = None;
                npc.home = me;
                let to = goal - me;
                npc.home_yaw = if npc.order_goal.is_none() {
                    to.y.atan2(to.x)
                } else {
                    npc.yaw
                };
                npc.orders_failed = false;
                return DVec3::ZERO;
            }
            let replan = npc
                .route
                .as_ref()
                .map_or(true, |r| r.age > 1.0 || r.goal.distance(goal) > 48.0);
            if replan {
                npc.route = Some(plan_route(level, me, goal, &p, None));
            }
            let r = npc.route.as_mut().unwrap();
            let w = follow(level, r, me, &p, dt);
            if w.length_squared() > 1e-6 {
                npc.yaw = turn_toward(npc.yaw, w.y.atan2(w.x), npc.yaw_rate * dt as f64);
            }
            w
        }
        Orders::Standing => {
            // Go back to where we were placed, then face the way we were facing.
            if flat_dist(me, npc.home) > 48.0 && !npc.orders_failed {
                if npc
                    .route
                    .as_ref()
                    .map_or(true, |r| r.goal.distance(npc.home) > 1.0)
                {
                    npc.route = Some(plan_route(level, me, npc.home, &p, None));
                }
                let r = npc.route.as_mut().unwrap();
                let w = follow(level, r, me, &p, dt);
                if r.stuck > 4 {
                    npc.orders_failed = true;
                }
                return w;
            }
            npc.route = None;
            npc.yaw = turn_toward(npc.yaw, npc.home_yaw, npc.yaw_rate * dt as f64);
            DVec3::ZERO
        }
        Orders::Wandering if npc.cleaner => {
            // CleanerBot.Wandering: PickDestination, MoveTo at walking speed, a second's pause if it gave up.
            if npc.wander_goal.is_none() {
                let (minor_mag, major_mag, min_dist) = (256.0, p.radius * 2.0, 24.0);
                let rev = |d: u8| match d {
                    0 => 1,
                    1 => 0,
                    2 => 3,
                    _ => 2,
                };
                let mut goal = None;
                'pick: {
                    if let Some(g) = direction_reachable(level, me, &p, npc.minor_dir, min_dist, minor_mag) {
                        goal = Some(g);
                        break 'pick;
                    }
                    npc.minor_dir = rev(npc.minor_dir);
                    if let Some(g) = direction_reachable(level, me, &p, npc.major_dir, min_dist, major_mag) {
                        goal = Some(g);
                        break 'pick;
                    }
                    npc.major_dir = rev(npc.major_dir);
                    if let Some(g) = direction_reachable(level, me, &p, npc.minor_dir, min_dist, minor_mag) {
                        goal = Some(g);
                        break 'pick;
                    }
                    npc.minor_dir = rev(npc.minor_dir);
                    if let Some(g) = direction_reachable(level, me, &p, npc.major_dir, min_dist, major_mag) {
                        goal = Some(g);
                        break 'pick;
                    }
                    npc.major_dir = rev(npc.major_dir);
                }
                npc.clean_time = 0.0;
                match goal {
                    Some(g) => npc.wander_goal = Some(g),
                    None => {
                        npc.pause = 1.0;
                        npc.pause_yaw = None;
                        return DVec3::ZERO;
                    }
                }
            }
            let goal = npc.wander_goal.unwrap();
            let d = (goal - me).truncate();
            npc.clean_time += dt;
            if d.length() < 6.0 || npc.clean_time > 8.0 {
                npc.wander_goal = None;
                return DVec3::ZERO;
            }
            let w = DVec3::new(d.x, d.y, 0.0).normalize_or_zero();
            npc.yaw = turn_toward(npc.yaw, w.y.atan2(w.x), npc.yaw_rate * dt as f64);
            w
        }
        Orders::Wandering if npc.animal => {
            // Animal / Rat Wandering.PickDestination: a random reachable spot 80-250 (rats) or 100-400 away,
            // kept within HomeExtent and in sight of home; then a short pause.
            if npc.wander_goal.is_none() {
                let rat = npc.alliance == "Rat";
                for _ in 0..4 {
                    let mut mag = if rat {
                        (170.0 * npc.rand() as f64 + 80.0) * (npc.rand() as f64 * 0.2 + 0.9)
                    } else {
                        (npc.wanderlust as f64 * 300.0 + 100.0) * (npc.rand() as f64 * 0.2 + 0.9)
                    };
                    let mut yaw = npc.rand() as f64 * std::f64::consts::TAU;
                    if npc.use_home {
                        if mag > npc.home_extent {
                            mag = npc.home_extent * (npc.rand() as f64 * 0.2 + 0.9);
                        }
                        // Biased back toward home (yaw spread 0.25 of a turn).
                        let to_home = (npc.home - me).truncate();
                        if to_home.length() > 1.0 {
                            yaw = to_home.y.atan2(to_home.x)
                                + (npc.rand() as f64 - 0.5) * 0.5 * std::f64::consts::TAU;
                        }
                    }
                    let dir = DVec3::new(yaw.cos(), yaw.sin(), 0.0);
                    let h = level.collision.trace(
                        me,
                        me + dir * mag,
                        DVec3::new(p.radius, p.radius, p.height * 0.5),
                    );
                    let dist = mag * h.fraction - p.radius;
                    if dist < 30.0 {
                        continue;
                    }
                    let goal = me + dir * dist;
                    if is_near_home(level, npc, goal) {
                        npc.wander_goal = Some(goal);
                        break;
                    }
                }
                if npc.wander_goal.is_none() {
                    npc.pause = 1.0 + npc.rand();
                    return DVec3::ZERO;
                }
            }
            let goal = npc.wander_goal.unwrap();
            let d = (goal - me).truncate();
            if d.length() < 12.0 {
                npc.wander_goal = None;
                npc.pause = 1.0 + npc.rand() * 3.0 * (1.0 - npc.restlessness * 0.5);
                npc.pause_yaw = None;
                return DVec3::ZERO;
            }
            DVec3::new(d.x, d.y, 0.0).normalize_or_zero()
        }
        Orders::Patrolling | Orders::Wandering => {
            if npc.patrol_dest.is_none() {
                npc.patrol_dest = if npc.orders == Orders::Patrolling {
                    level.nav.patrol_start(&npc.order_tag, [me.x, me.y, me.z])
                } else {
                    // Wander: a random path point near home (within HomeExtent and in sight of it).
                    let near: Vec<usize> = level
                        .nav
                        .nearby(
                            [npc.home.x, npc.home.y, npc.home.z],
                            if npc.use_home { npc.home_extent } else { 900.0 },
                        )
                        .into_iter()
                        .filter(|&i| {
                            let q = level.nav.points[i].pos;
                            is_near_home(level, npc, DVec3::new(q[0], q[1], q[2]))
                        })
                        .collect();
                    if near.is_empty() {
                        None
                    } else {
                        let k = (npc.rand() * near.len() as f32) as usize;
                        Some(near[k.min(near.len() - 1)])
                    }
                };
                if npc.patrol_dest.is_none() {
                    npc.orders = Orders::Standing;
                    return DVec3::ZERO;
                }
                npc.route = None;
            }
            let dest = npc.patrol_dest.unwrap();
            let q = level.nav.points[dest].pos;
            let goal = DVec3::new(q[0], q[1], q[2]);
            if npc.route.is_none() {
                npc.route = Some(plan_route(level, me, goal, &p, Some(dest)));
            }
            let r = npc.route.as_mut().unwrap();
            let w = follow(level, r, me, &p, dt);
            let arrived = (r.nodes.is_empty() && flat_dist(me, goal) < 24.0) || r.stuck > 6;
            if arrived {
                npc.route = None;
                let pt = &level.nav.points[dest];
                if npc.orders == Orders::Patrolling {
                    // Pause where the point says (or at the end of the chain), facing its direction.
                    if pt.pause_time > 0.0 || pt.next_patrol.is_none() {
                        npc.pause = pt.pause_time * (1.0 - 0.9 * npc.restlessness);
                        npc.pause_yaw = Some(pt.yaw as f64 / 65536.0 * std::f64::consts::TAU);
                    }
                    npc.patrol_dest = pt.next_patrol;
                    if npc.patrol_dest.is_none() {
                        npc.orders = Orders::Standing;
                        npc.home = me;
                        npc.home_yaw = npc.pause_yaw.unwrap_or(npc.yaw);
                    }
                } else {
                    npc.pause = 2.0 + npc.rand() * 5.0 * (1.0 - npc.restlessness * 0.5);
                    npc.pause_yaw = None;
                    npc.patrol_dest = None;
                }
                return DVec3::ZERO;
            }
            w
        }
    }
}

/// ScriptedPawn state Seeking: go to where the enemy was seen or heard (sight: just turn to look), look around
/// for 3 to 4.5 seconds and then a little longer, poke around nearby (SeekLevel more times), then give up and go
/// back to orders. Returns (wish direction, running).
fn do_seek(
    level: &dx_level::Level,
    npc: &mut Npc,
    me: &Cylinder,
    my_eye: DVec3,
    world: &World,
    dt: f32,
    now: f32,
) -> (DVec3, bool) {
    let p = npc.params.clone();
    let here = me.center;
    let Some(mut s) = npc.seek.clone() else {
        npc.finish_seek();
        return (DVec3::ZERO, false);
    };
    let mut wish = DVec3::ZERO;
    let mut running = false;
    // PickDestination: the remembered spot, or somewhere 130-250 units away while SeekLevel lasts.
    let pick = |npc: &mut Npc, s: &mut Seek| -> bool {
        if npc.seek_level <= 0 {
            return false;
        }
        if s.located {
            s.dest = s.pos;
            return true;
        }
        for _ in 0..8 {
            let yaw = npc.rand() as f64 * std::f64::consts::TAU;
            let d = 130.0 + npc.rand() as f64 * 120.0;
            let to = here + DVec3::new(yaw.cos(), yaw.sin(), 0.0) * d;
            if clear_line(level, here, to, p.radius) {
                s.dest = to;
                return true;
            }
        }
        let yaw = npc.rand() as f64 * std::f64::consts::TAU;
        s.dest = here + DVec3::new(yaw.cos(), yaw.sin(), 0.0) * 50.0;
        true
    };
    let start_turn = |npc: &mut Npc, s: &mut Seek| {
        // A guess faces a random way roughly ahead.
        if s.kind == SeekKind::Guess && s.located {
            let yaw = npc.yaw + (npc.rand() as f64 - 0.5) * std::f64::consts::FRAC_PI_2;
            s.dest = here + DVec3::new(yaw.cos(), yaw.sin(), 0.0) * 1000.0;
        }
        s.phase = SeekPhase::Turn;
        npc.route = None;
    };
    match s.phase {
        SeekPhase::Begin => {
            if pick(npc, &mut s) {
                s.phase = SeekPhase::Go;
            } else {
                npc.finish_seek();
                return (DVec3::ZERO, false);
            }
        }
        SeekPhase::Go => {
            if s.kind == SeekKind::Sight && s.located {
                start_turn(npc, &mut s);
            } else {
                // GetNextLocation: done once within seekDistance with a clear view of the spot.
                let elapsed = (now - npc.last_seen).max(0.0) as f64;
                let seek_dist = if s.located {
                    if s.kind == SeekKind::Guess {
                        200.0 + (p.ground_speed * elapsed * 0.5).clamp(0.0, 1000.0)
                    } else {
                        300.0
                    }
                } else {
                    60.0
                };
                let dist = (here - s.dest).length();
                let in_view = dist < seek_dist
                    && world.trace(my_eye, s.dest, DVec3::splat(1.0)).fraction >= 0.999;
                let stuck = npc.route.as_ref().map_or(false, |r| r.stuck > 3);
                if in_view || stuck || dist < 24.0 {
                    if s.kind == SeekKind::Guess && s.located {
                        // GetOvershootDestination / AIPickRandomDestination: carry on a little past it.
                        let mut over = None;
                        for _ in 0..6 {
                            let yaw = npc.yaw + (npc.rand() as f64 - 0.5) * 1.5;
                            let to = here
                                + DVec3::new(yaw.cos(), yaw.sin(), 0.0)
                                    * (200.0 + npc.rand() as f64 * 200.0);
                            if clear_line(level, here, to, p.radius) {
                                over = Some(to);
                                break;
                            }
                        }
                        npc.route = None;
                        match over {
                            Some(o) => s.phase = SeekPhase::Overshoot(o),
                            None => start_turn(npc, &mut s),
                        }
                    } else {
                        start_turn(npc, &mut s);
                    }
                } else {
                    if npc
                        .route
                        .as_ref()
                        .map_or(true, |r| r.goal.distance(s.dest) > 1.0 || r.age > 2.0)
                    {
                        let stuck = npc.route.as_ref().map(|r| r.stuck).unwrap_or(0);
                        let mut r = plan_route(level, here, s.dest, &p, None);
                        r.stuck = stuck;
                        npc.route = Some(r);
                    }
                    let r = npc.route.as_mut().unwrap();
                    wish = follow(level, r, here, &p, dt);
                    running = true;
                }
            }
        }
        SeekPhase::Overshoot(o) => {
            if npc.route.is_none() {
                npc.route = Some(plan_route(level, here, o, &p, None));
            }
            let r = npc.route.as_mut().unwrap();
            let w = follow(level, r, here, &p, dt);
            if flat_dist(here, o) < 20.0 || r.stuck > 2 || w.length_squared() < 1e-6 {
                start_turn(npc, &mut s);
            } else {
                wish = w;
                running = true;
            }
        }
        SeekPhase::Turn => {
            let to = s.dest - here;
            let want = to.y.atan2(to.x);
            npc.yaw = turn_toward(npc.yaw, want, npc.yaw_rate * dt as f64);
            let off = (want - npc.yaw).sin().abs() < 0.08 && (want - npc.yaw).cos() > 0.0;
            if off || to.truncate().length() < 1.0 {
                s.located = false;
                s.phase = SeekPhase::Wait(npc.rand() * 1.5 + 3.0);
            }
        }
        SeekPhase::Wait(t) => {
            let t = t - dt;
            s.phase = if t > 0.0 {
                SeekPhase::Wait(t)
            } else {
                SeekPhase::Look(3.0)
            };
        }
        SeekPhase::Look(t) => {
            // LookAround's head turns are skipped once there's somewhere new to look.
            let t = if s.located { 0.0 } else { t - dt };
            if t > 0.0 {
                s.phase = SeekPhase::Look(t);
            } else {
                // FindAnotherPlace.
                npc.seek_level -= 1;
                if pick(npc, &mut s) {
                    s.phase = SeekPhase::Go;
                    npc.route = None;
                } else {
                    npc.finish_seek();
                    return (DVec3::ZERO, false);
                }
            }
        }
    }
    npc.seek = Some(s);
    (wish, running)
}

/// Pawn.AIDirectionReachable(Location, Yaw, 0, minDist, maxDist, bestDest) for CleanerBot's ECleanDirection
/// (North = yaw 0 = +X, South, East = +Y, West): how far the pawn can walk that way, up to `max` - stopped by
/// walls and by ledges (no floor within MaxStepHeight below its feet); reachable if at least `min`.
fn direction_reachable(
    level: &dx_level::Level,
    me: DVec3,
    p: &imsim_core::movement::MoveParams,
    dir: u8,
    min: f64,
    max: f64,
) -> Option<DVec3> {
    let v = match dir {
        0 => DVec3::X,
        1 => DVec3::NEG_X,
        2 => DVec3::Y,
        _ => DVec3::NEG_Y,
    };
    direction_reachable_v(level, me, p, v, min, max)
}

/// AIDirectionReachable along any flat direction: how far (min..max) the pawn can walk that way on floor.
fn direction_reachable_v(
    level: &dx_level::Level,
    me: DVec3,
    p: &imsim_core::movement::MoveParams,
    v: DVec3,
    min: f64,
    max: f64,
) -> Option<DVec3> {
    let v = v.truncate().extend(0.0).normalize_or_zero();
    if v.length_squared() < 1e-9 {
        return None;
    }
    let ext = DVec3::new(p.radius, p.radius, p.height);
    // Lifted by the step height so a small lip doesn't count as a wall.
    let up = DVec3::new(0.0, 0.0, p.max_step_height.min(p.height));
    let h = level.collision.trace(me + up, me + up + v * max, ext);
    let mut reach = (max * h.fraction - 1.0).max(0.0);
    // Walk it in 8-unit steps looking for the floor.
    let mut d = 8.0;
    let mut last = 0.0;
    while d <= reach {
        let at = me + v * d;
        let down = level.collision.trace(
            at,
            at - DVec3::new(0.0, 0.0, p.max_step_height + 4.0),
            DVec3::new(p.radius * 0.5, p.radius * 0.5, p.height),
        );
        if down.fraction >= 1.0 {
            break;
        }
        last = d;
        d += 8.0;
    }
    reach = reach.min(last);
    (reach >= min).then(|| me + v * reach)
}

/// Living CleanerBots this frame: collision centre, radius, height (effects::cleaner_system wipes decals).
pub static CLEANERS: std::sync::Mutex<Vec<(DVec3, f64, f64)>> = std::sync::Mutex::new(Vec::new());

/// Sitting orders: move onto the seat tagged `OrderTag` (ScriptedPawn Sitting.SitPosition).
fn sit_down(level: &dx_level::Level, npc: &mut Npc, deco: &mut Deco, p: &[Property]) {
    let ordered = level.actors.iter().position(|a| {
        !npc.order_tag.is_empty()
            && name_of(&a.props, "Tag").eq_ignore_ascii_case(&npc.order_tag)
            && find(&a.props, "sitPoint").is_some()
    });
    // FindBestSeat: no seat ordered, so the nearest free one (the original looks 10000 units out and walks there;
    // a pawn is placed on one close by).
    let seat_i = ordered.or_else(|| {
        let taken = TAKEN.lock().map(|t| t.clone()).unwrap_or_default();
        level
            .actors
            .iter()
            .enumerate()
            .filter(|(i, a)| find(&a.props, "sitPoint").is_some() && !taken.contains(i))
            .map(|(i, a)| {
                let l = a.location();
                (i, DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64).distance(npc.home))
            })
            .filter(|(_, d)| *d < 600.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    let Some(seat_i) = seat_i else {
        npc.orders = Orders::Standing;
        return;
    };
    if let Ok(mut t) = TAKEN.lock() {
        t.push(seat_i);
    }
    let seat = &level.actors[seat_i];
    let sp = match find(&seat.props, "sitPoint") {
        Some(Value::Vector(v)) => *v,
        _ => [0.0; 3],
    };
    let rot = match find(&seat.props, "Rotation") {
        Some(Value::Rotator(r)) => *r,
        _ => [0; 3],
    };
    let axes = dx_level::actors::rotator_axes(rot);
    let rp = [
        axes[0][0] * sp[0] + axes[1][0] * sp[1] + axes[2][0] * sp[2],
        axes[0][1] * sp[0] + axes[1][1] * sp[1] + axes[2][1] * sp[2],
        axes[0][2] * sp[0] + axes[1][2] * sp[1] + axes[2][2] * sp[2],
    ];
    let l = seat.location();
    let ass = fv(p, "BaseAssHeight", -26.0) as f64;
    // Sitting collision height is default + BaseAssHeight/2, so the cylinder centre sits this far from the seat.
    let pos = DVec3::new(
        (l[0] + rp[0]) as f64,
        (l[1] + rp[1]) as f64,
        (l[2] + rp[2]) as f64 - ass * 0.5,
    );
    deco.body.location = pos;
    deco.body.physics = imsim_core::body::BodyPhysics::None;
    // SitPosition is taken from the seat where it is now, and the pawn is based on it: decorations fall into
    // place at level start (map chairs are often saved above the floor), so follow the seat's live position.
    npc.seat = Some((seat_i, None, pos - DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)));
    // SetBasedPawnSize(CollisionRadius, GetSitHeight()): the cylinder shrinks to GetDefaultCollisionHeight() +
    // BaseAssHeight/2 and PrePivot grows by the difference, so the mesh is drawn Default.CollisionHeight - sit height
    // above the (lowered) collision centre - without this the body sinks into the seat by ~17 units. Standing up
    // goes back through the crouch resize below (height -> stand_height, PrePivot back).
    let sit_h = (npc.stand_height - 4.5 + ass * 0.5).max(1.0);
    deco.pre.z += npc.params.height - sit_h;
    npc.params.height = sit_h;
    deco.body.height = sit_h;
    npc.yaw = (rot[1] - 16384) as f64 / 65536.0 * std::f64::consts::TAU;
    npc.home = pos;
    npc.home_yaw = npc.yaw;
    npc.sitting = true;
}

#[allow(clippy::too_many_arguments)]
pub fn npc_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mv: Res<Movers>,
    player: Res<Player>,
    mut ph: ResMut<PlayerHealth>,
    mut hp: ResMut<PawnHealth>,
    mut attackers: ResMut<LastAttacker>,
    _hud: Res<HudMessage>,
    mut assets: ResMut<RuntimeAssets>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<Npc>)>,
    mut q: Query<(&mut Npc, &mut Deco, &mut Animated, &mut Transform), Without<OutOfWorld>>,
    walk_test: Option<Res<crate::WalkTest>>,
    mut test_clock: Local<f32>,
    (
        mut deco_hits,
        mut pawn_hits,
        mut noises,
        plight,
        carrying,
        mut launches,
        mut blood,
        mut drips,
    ): (
        ResMut<crate::destruct::DecoHits>,
        ResMut<crate::damage::PawnHits>,
        ResMut<Noises>,
        Res<crate::stealth::PlayerLight>,
        Res<crate::carry::Carrying>,
        ResMut<crate::projectiles::Launches>,
        ResMut<crate::effects::BloodFx>,
        ResMut<crate::effects::BloodDrips>,
    ),
    (mut booms, mut gibs, mut weapon_drops, mut alarms, mut flags, mut takedowns): (
        ResMut<crate::explosion::Explosions>,
        ResMut<crate::effects::Gibs>,
        ResMut<WeaponDrops>,
        ResMut<crate::alarms::Alarms>,
        ResMut<GameFlags>,
        ResMut<Takedowns>,
    ),
) {
    let _prof = crate::prof::scope("ai::npc_system");
    let danger: Vec<(DVec3, f64, Option<usize>)> = crate::grenades::DANGER
        .lock()
        .map(|d| d.clone())
        .unwrap_or_default();
    // Everything heard since last frame (footsteps, shots, landings, cries); pawns add their own for next frame.
    let heard: Vec<Noise> = noises.0.drain(..).collect();
    let stim = crate::reactions::take();
    let mut new_noises: Vec<Noise> = Vec::new();
    let mut hits_by: std::collections::HashMap<usize, Vec<crate::damage::PawnHit>> =
        std::collections::HashMap::new();
    for h in pawn_hits.0.drain(..) {
        hits_by.entry(h.blocker).or_default().push(h);
    }
    let mut new_hits: Vec<crate::damage::PawnHit> = Vec::new();
    // Automated tests run the simulation at a fixed 60 Hz regardless of frame rate.
    let dt = if walk_test.is_some() {
        1.0 / 60.0
    } else {
        time.delta_secs().min(0.1)
    };
    *test_clock += dt;
    let now = if walk_test.is_some() {
        *test_clock
    } else {
        time.elapsed_secs()
    };
    let lvl = &level.0;
    let eye_b = cam.translation;
    let _player_eye = {
        let u = dx_level::bevy_to_ue(eye_b.to_array());
        DVec3::new(u[0] as f64, u[1] as f64, u[2] as f64)
    };
    let player_center = player.body.location;
    let player_yaw = {
        let f = cam.forward().as_vec3();
        (f.z as f64).atan2(f.x as f64)
    };
    let player_cyl = Cylinder {
        center: player_center,
        radius: player.params.radius,
        height: player.params.height,
        actor: usize::MAX,
    };
    let pose = std::env::var("DXVIEW_POSE").ok();

    // Snapshot of pawns for target selection and distress calls.
    struct Snap {
        blocker: usize,
        alliance: String,
        dead: bool,
        target: Option<Target>,
        last_seen: f32,
        enemy_timeout: f32,
        alliances: imsim_core::alliance::AllianceTable,
        pos: DVec3,
        emit_carcass: bool,
        /// Any body (dead or unconscious, not a robot) - for the Bodies Raise Alarms enhancement.
        any_body: bool,
        killer_alliance: String,
        on_ground: bool,
        vel: DVec3,
        height: f64,
        bind: String,
    }
    let pawns: Vec<Snap> = q
        .iter()
        .map(|(n, d, _, _)| Snap {
            blocker: n.blocker,
            alliance: n.alliance.clone(),
            dead: n.state == NpcState::Dead,
            target: n.target,
            last_seen: n.last_seen,
            enemy_timeout: n.enemy_timeout,
            alliances: n.alliances.clone(),
            pos: d.body.location,
            emit_carcass: n.emit_carcass && !n.carried,
            any_body: n.state == NpcState::Dead && !n.carried && !n.hp_body.robot && !n.animal,
            killer_alliance: n.killer_alliance.clone(),
            on_ground: d.body.physics != imsim_core::body::BodyPhysics::Falling,
            vel: d.body.velocity,
            height: d.body.height,
            bind: n.bind_name.clone(),
        })
        .collect();
    let snap_of = |b: usize| pawns.iter().find(|p| p.blocker == b);
    // GetPawnAllianceType == Hostile: either side hostile to the other's alliance.
    let snap_of_hostile = |npc: &Npc, t: Target| -> bool {
        match t {
            Target::Player => npc.hates("Player"),
            Target::Pawn(b) => snap_of(b)
                .is_some_and(|p| npc.hates(&p.alliance) || p.alliances.hostile_to(&npc.alliance)),
        }
    };

    let ai_log = walk_test.is_some() || std::env::var("DXVIEW_AILOG").is_ok();
    // Orders wait while their pawn is in a conversation (ConvOrders, applied when it ends).
    let pawn_orders = crate::triggers::take_orders();
    let busy_tags: std::collections::HashSet<String> = q
        .iter()
        .filter(|(n, _, _, _)| {
            n.state != NpcState::Dead && crate::consys::conversing(n.blocker).is_some()
        })
        .map(|(n, _, _, _)| actor_tag(&lvl.actors[n.actor]).to_ascii_lowercase())
        .collect();
    crate::triggers::defer_orders(
        pawn_orders
            .iter()
            .filter(|o| matches!(o, crate::triggers::PawnOrder::Orders { tag, .. } if busy_tags.contains(&tag.to_ascii_lowercase())))
            .cloned()
            .collect(),
    );
    if let Ok(mut c) = CLEANERS.lock() {
        c.clear();
    }
    for (mut npc, mut deco, mut an, mut tf) in &mut q {
        let npc = &mut *npc;
        npc.just_fired = false;
        npc.weapon_timer += dt;
        let state_before = std::mem::discriminant(&npc.state);
        if npc.state == NpcState::Dead {
            continue;
        }
        // GuntherHermann KillswitchActivated: HitTorso / HitHead / HitTorsoBack / HitHeadBack at 2x, then the head
        // hits at 3x and 5x, then Explode + Destroy.
        if let Some(t) = npc.killswitch.as_mut() {
            *t += dt;
            const SEQ: [(&str, f32); 8] = [
                ("HitTorso", 2.0),
                ("HitHead", 2.0),
                ("HitTorsoBack", 2.0),
                ("HitHeadBack", 2.0),
                ("HitHead", 3.0),
                ("HitHeadBack", 3.0),
                ("HitHead", 5.0),
                ("HitHeadBack", 5.0),
            ];
            let mut at = 0.0;
            let mut cur = None;
            for (i, (_, rate)) in SEQ.iter().enumerate() {
                let len = 0.8 / rate;
                if *t < at + len {
                    cur = Some(i);
                    break;
                }
                at += len;
            }
            match cur {
                Some(i) => {
                    let ma = &lvl.mesh_actors[an.mesh_actor];
                    let key = SEQ[i].0;
                    if npc.anim != key || an.clip.as_ref().is_some_and(|c| c.done()) {
                        if let Some(anim) = &ma.anim {
                            if let Some(mut c) = Clip::named(&anim.mesh, key, false) {
                                c.rate *= SEQ[i].1;
                                an.clip = Some(c);
                                npc.anim = key;
                            }
                        }
                    }
                }
                None => {
                    npc.killswitch = None;
                    let c = blockers.list[npc.blocker].center;
                    new_hits.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: 10000.0,
                        damage_type: "Shot",
                        hit_loc: c,
                        attacker: Some(npc.blocker),
                        attacker_pos: c,
                    });
                }
            }
            continue;
        }
        let con_look = crate::consys::conversing(npc.blocker);
        let standing = npc.orders == Orders::Standing && npc.route.is_none() && con_look.is_none();
        npc.barks(dt, standing);
        let con_hold = con_look.is_some() && crate::consys::third_person();
        // LipSynch runs while the pawn is in a conversation (state Conversation / FirstPersonConversation).
        an.face.talking = con_look.is_some();
        an.face.phoneme = if con_look.is_some() {
            crate::consys::mouth(crate::consys::Who::Actor(npc.blocker))
        } else {
            None
        };
        // OrdersTrigger / AllianceTrigger (and conversations' triggers) for pawns tagged so.
        let my_tag = actor_tag(&lvl.actors[npc.actor]);
        for o in &pawn_orders {
            match o {
                crate::triggers::PawnOrder::Orders {
                    tag,
                    orders,
                    orders_tag,
                } if tag.eq_ignore_ascii_case(&my_tag)
                    && !busy_tags.contains(&my_tag.to_ascii_lowercase()) =>
                {
                    npc.set_orders(lvl, orders, orders_tag, player_center, now);
                }
                crate::triggers::PawnOrder::Script { sel, orders, orders_tag }
                    if sel_hit(sel, &lvl.actors[npc.actor].class, &npc.bind_name, &my_tag) =>
                {
                    npc.set_orders(lvl, orders, orders_tag, player_center, now);
                }
                crate::triggers::PawnOrder::Ally { sel, alliance, level, permanent }
                    if sel_hit(sel, &lvl.actors[npc.actor].class, &npc.bind_name, &my_tag) =>
                {
                    npc.alliances.change_ally(alliance, *level, *permanent, false);
                    if !npc.hates("Player") && npc.target == Some(Target::Player) {
                        npc.target = None;
                    }
                }
                crate::triggers::PawnOrder::Alliance {
                    tag,
                    alliance,
                    list,
                } if tag.eq_ignore_ascii_case(&my_tag) => {
                    if !alliance.is_empty() {
                        npc.alliance = alliance.clone();
                        npc.alliances.own = alliance.clone();
                        npc.alliances.change_ally(alliance, 1.0, true, false);
                    }
                    for (n, l, perm) in list {
                        npc.alliances.change_ally(n, *l, *perm, false);
                    }
                    if !npc.hates("Player") && npc.target == Some(Target::Player) {
                        npc.target = None;
                        if npc.state == NpcState::Attacking
                            || matches!(npc.state, NpcState::Surprised(_))
                        {
                            npc.state = NpcState::Idle;
                        }
                    }
                }
                _ => {}
            }
        }
        // Lazy setup: weapon from InitialInventory, seat, walking body once the pawn has landed.
        if !npc.inited {
            npc.inited = true;
            let a = &lvl.actors[npc.actor];
            // InitializeInventory: Count copies of each entry; ammo adds Count x AmmoAmount; a weapon whose AmmoName
            // isn't carried gets a default box of it.
            for x in a.props.iter().filter(|x| x.name == "InitialInventory") {
                if let Value::Item(path, count) = &x.value {
                    let class = path.rsplit('.').next().unwrap_or("");
                    if *count <= 0 || class.is_empty() || class == "None" {
                        continue;
                    }
                    if class.starts_with("Ammo") {
                        if npc.ammo.is_none() {
                            npc.ammo = Some(class.to_string());
                        }
                        let per = match find(&assets.classes.defaults(class), "AmmoAmount") {
                            Some(Value::Int(n)) => *n,
                            _ => 1,
                        };
                        match npc.ammo_left.iter_mut().find(|e| e.0 == class) {
                            Some(e) => e.1 += per * count,
                            None => npc.ammo_left.push((class.to_string(), per * count)),
                        }
                    }
                    if class.starts_with("Weapon") && !npc.arsenal.iter().any(|w| w.class == class)
                    {
                        let d = assets.classes.defaults(class);
                        if let Some(w) = weapon_stats(class, &d) {
                            npc.arsenal.push(w);
                        }
                    }
                }
            }
            for i in 0..npc.arsenal.len() {
                let w = npc.arsenal[i].clone();
                if let Some(a) = w.ammo_class.clone() {
                    if npc.ammo_class_for(&w).as_deref() == Some(a.as_str())
                        && !npc.ammo_left.iter().any(|e| e.0 == a)
                    {
                        let per = match find(&assets.classes.defaults(&a), "AmmoAmount") {
                            Some(Value::Int(n)) => *n,
                            _ => 1,
                        };
                        npc.ammo_left.push((a, per));
                    }
                }
            }
            // SetupWeapon: the first weapon given is the one drawn until a fight picks the best.
            if let Some(w) = npc.arsenal.first().cloned() {
                let left = npc
                    .ammo_class_for(&w)
                    .map(|a| npc.ammo_count(&a))
                    .unwrap_or(i32::MAX);
                npc.loaded = if w.reload_count == 0 {
                    1
                } else {
                    (w.reload_count as i32).min(left).max(0) as u32
                };
                npc.weapon = Some(w);
                npc.switch_to_best_weapon(None);
            }
            if npc.orders == Orders::Sitting {
                sit_down(lvl, npc, &mut deco, &a.props);
                if npc.sitting {
                    blockers.list[npc.blocker].height = npc.params.height;
                }
            }
            if a.class == "Gray" {
                npc.aura = Some((
                    fv(&a.props, "DamageRadius", 256.0) as f64,
                    fv(&a.props, "damageInterval", 1.0),
                    fv(&a.props, "DamageAmount", 10.0),
                ));
            }
        }
        if npc.body.is_none() {
            if deco.body.physics == imsim_core::body::BodyPhysics::None {
                let mut c = Character::new(deco.body.location);
                c.physics = if npc.sitting {
                    Physics::Noclip
                } else {
                    Physics::Walking
                };
                npc.body = Some(c);
                npc.home = deco.body.location;
            } else {
                continue;
            }
        }

        // Damage taken since last frame (shots, prods), and the reaction to it.
        npc.take_hit_timer = (npc.take_hit_timer - dt).max(0.0);
        // Timer while burning: WeaponFlamethrower.BurnDamage (5) 'Burned' a second for burnTime (30 s).
        if let Some(t) = ph.on_fire {
            if ph.dead {
                ph.on_fire = None;
            } else {
                ph.burn_tick += dt;
                if ph.burn_tick >= 1.0 {
                    ph.burn_tick -= 1.0;
                    ph.damage(5.0, "Burned", DVec3::ZERO);
                }
                ph.on_fire = if t + dt >= 30.0 { None } else { Some(t + dt) };
            }
        }
        // UpdatePoison.
        if npc.poison_counter > 0 {
            npc.poison_timer += dt;
            if npc.poison_timer >= 2.0 {
                npc.poison_timer = 0.0;
                npc.poison_counter -= 1;
                let c = blockers.list[npc.blocker].center;
                new_hits.push(crate::damage::PawnHit {
                    blocker: npc.blocker,
                    damage: npc.poison_damage,
                    damage_type: "PoisonEffect",
                    hit_loc: c,
                    attacker: npc.poisoner,
                    attacker_pos: c,
                });
            }
        }
        // GMDX UpdateBleed: a BleedEffect tick every second, with a spray of blood drops.
        if npc.bleed_counter > 0 {
            npc.bleed_timer += dt;
            if npc.bleed_timer >= 1.0 {
                npc.bleed_timer = 0.0;
                npc.bleed_counter -= 1;
                let c = blockers.list[npc.blocker].center;
                if npc.bleed_damage > 0.0 {
                    new_hits.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: npc.bleed_damage,
                        damage_type: "BleedEffect",
                        hit_loc: c,
                        attacker: None,
                        attacker_pos: c,
                    });
                }
            }
        }
        if let Some(list) = hits_by.remove(&npc.blocker) {
            let center = blockers.list[npc.blocker].center;
            for h in list {
                // FilterDamageType overrides: Karkians, Greasels and Grays shrug off gas; Grays also radiation and
                // fire. Animal.ModifyDamage zeroes 'Stunned' (prods don't work on animals).
                let cls = lvl.actors[npc.actor].class.as_str();
                if matches!(cls, "Karkian" | "KarkianBaby" | "Greasel" | "Gray")
                    && matches!(h.damage_type, "TearGas" | "HalonGas" | "PoisonGas")
                {
                    continue;
                }
                if cls == "Gray" && matches!(h.damage_type, "Radiation" | "Flamed" | "Burned") {
                    continue;
                }
                if npc.animal && h.damage_type == "Stunned" {
                    // The prod does no damage to animals; ordinary ones (not transgenics) run from it.
                    if !matches!(cls, "Karkian" | "KarkianBaby" | "Greasel" | "Gray") {
                        let away = (center - h.attacker_pos).truncate().normalize_or_zero();
                        let away = if away.length_squared() < 1e-6 {
                            DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0)
                        } else {
                            DVec3::new(away.x, away.y, 0.0)
                        };
                        npc.avoiding = 3.0;
                        npc.flee_dest = Some(center + away * 400.0);
                        npc.route = None;
                    }
                    continue;
                }
                // Class overrides of ShieldDamage / ModifyDamage / IgnoreDamageType (bosses and commandos).
                let mut h = h;
                {
                    let scale = crate::pawnclass::damage_scale(cls, h.damage_type);
                    if scale <= 0.0 {
                        continue;
                    }
                    h.damage *= scale;
                }
                let stunned = matches!(npc.state, NpcState::Stunned(_) | NpcState::RubbingEyes(_));
                // EMP: robots lose EMPHitPoints and shut down (Robot state Disabled) at zero; people don't care.
                if h.damage_type == "EMP" {
                    if npc.hp_body.robot && npc.emp_hp > 0.0 {
                        npc.emp_hp -= h.damage;
                        if npc.emp_hp <= 0.0 {
                            npc.emp_hp = 0.0;
                            npc.state = NpcState::Stunned(1.0e9);
                            npc.target = None;
                            npc.anim = "";
                            println!("PAWN {} disabled by EMP", lvl.actors[npc.actor].name);
                        }
                    }
                    continue;
                }
                // Tear gas: no damage, but the pawn turns on whoever gassed it and goes to RubbingEyes
                // (GotoDisabledState). Robots don't care.
                // FilterDamageType: halon puts a burning pawn out; then it's treated like tear gas.
                if h.damage_type == "HalonGas" {
                    npc.on_fire = None;
                }
                if h.damage_type == "TearGas" || h.damage_type == "HalonGas" {
                    if npc.hp_body.robot
                        || matches!(npc.state, NpcState::RubbingEyes(_))
                        || crate::pawnclass::ignores_gas_reaction(cls)
                    {
                        continue;
                    }
                    // ReactToInjury: a provocation; the gasser only becomes the enemy if that makes them hostile.
                    let (who, alliance) = match h.attacker {
                        None => (Target::Player, "Player".to_string()),
                        Some(b) => (
                            Target::Pawn(b),
                            snap_of(b).map(|p| p.alliance.clone()).unwrap_or_default(),
                        ),
                    };
                    if npc.hate_injury {
                        npc.alliances.provoke(&alliance, None);
                    }
                    if npc.alliances.hostile_to(&alliance)
                        || (who != Target::Player && snap_of_hostile(npc, who))
                    {
                        npc.target = Some(who);
                        npc.last_pos = target_pos(who, player_center, &blockers.list);
                        npc.last_seen = now;
                    }
                    npc.stand_up();
                    npc.weapon_drawn = false;
                    npc.route = None;
                    npc.seek = None;
                    let ma = &lvl.mesh_actors[an.mesh_actor];
                    let start = ma
                        .anim
                        .as_ref()
                        .and_then(|a| Clip::named(&a.mesh, "RubEyesStart", false));
                    let start_len = start
                        .as_ref()
                        .map(|c| (c.frames as f32 - 1.0).max(1.0) / c.rate.max(1.0))
                        .unwrap_or(0.0);
                    if let Some(c) = start {
                        an.clip = Some(c);
                        npc.anim = "RubEyesStart";
                    }
                    npc.state = NpcState::RubbingEyes(15.0 + start_len);
                    println!("PAWN {} gassed", lvl.actors[npc.actor].name);
                    continue;
                }
                // Robot.TakeDamageBase, NanoVirus (scramble grenade): CrazedTimer += damage / 2 - its alliances are
                // reversed until it runs out.
                if npc.hp_body.robot && h.damage_type == "NanoVirus" {
                    npc.crazed += 0.5 * h.damage;
                    npc.target = None;
                    println!("PAWN {} scrambled for {:.0} s", lvl.actors[npc.actor].name, npc.crazed);
                    continue;
                }
                let health_before = npc.hp_body.health;
                let out = npc
                    .hp_body
                    .take(&h, center, npc.yaw, stunned, npc.take_hit_timer <= 0.0);
                // TakeDamageBase: bleedRate += damage taken / (0.3 x default health), for wounds that bleed.
                if npc.can_bleed
                    && !npc.hp_body.robot
                    && !matches!(
                        h.damage_type,
                        "Stunned"
                            | "TearGas"
                            | "HalonGas"
                            | "PoisonGas"
                            | "Radiation"
                            | "EMP"
                            | "NanoVirus"
                            | "Drowned"
                            | "KnockedOut"
                            | "Poison"
                            | "PoisonEffect"
                    )
                {
                    npc.bleed_rate += (health_before - npc.hp_body.health).max(0.0)
                        / (0.3 * npc.hp_body.max_health.max(1.0));
                }
                hp.0.insert(npc.blocker, npc.hp_body.health);
                attackers.0.insert(npc.blocker, h.attacker);
                match out {
                    crate::damage::Outcome::Ignored => {}
                    crate::damage::Outcome::Died {
                        knocked_out,
                        fall_back,
                    } => {
                        // MIB / WIB / Anna / Gunther: SpawnCarcass explodes them unless they're stunned (knocked
                        // out bodies stay): 100 damage over 256, LoudNoise, explosions, scorch, rock and flesh chips.
                        let self_destruct = crate::pawnclass::explodes_on_death(cls) && !(knocked_out || stunned);
                        if self_destruct {
                            crate::sound::play("DeusExSounds.Generic.LargeExplosion1", center, 1.0, 256.0 * 16.0, 1.0);
                            booms.0.push(crate::explosion::Boom {
                                pos: center,
                                damage: 100.0,
                                radius: 256.0,
                                damage_type: "Exploded",
                                from: Some(npc.blocker),
                                rings: false,
                                fireball: true,
                                fx: 0,
                            });
                        }
                        // Robots blow up (Robot.Explode: HurtRadius(0.5 r, 8 r) with r = (radius + height) / 2);
                        // anyone else killed by an explosion is gibbed (Health < -100).
                        if npc.hp_body.robot || h.damage_type == "Exploded" || self_destruct {
                            let r = (npc.params.radius + npc.params.height) / 2.0;
                            if npc.hp_body.robot {
                                // Robot.Explode: explosionSound (volume 2) and a large explosion, out to r x 32.
                                let rr = (r * 32.0) as f32;
                                if let Some(snd) = crate::sound::prop(
                                    &lvl.actors[npc.actor].props,
                                    "explosionSound",
                                ) {
                                    crate::sound::play(&snd, center, 2.0, rr, 1.0);
                                }
                                crate::sound::play(
                                    if r < 48.0 {
                                        "DeusExSounds.Generic.LargeExplosion1"
                                    } else {
                                        "DeusExSounds.Generic.LargeExplosion2"
                                    },
                                    center,
                                    1.0,
                                    rr,
                                    1.0,
                                );
                                booms.0.push(crate::explosion::Boom {
                                    pos: center,
                                    damage: (0.5 * r) as f32,
                                    radius: 8.0 * r,
                                    damage_type: "Exploded",
                                    from: None,
                                    rings: false,
                                    fireball: true,
                                    fx: 0,
                                });
                            }
                            gibs.0.push((
                                center,
                                npc.params.radius,
                                npc.params.height,
                                npc.hp_body.robot,
                            ));
                            // GMDX fix (G-Flex / Shifter; setting FixGibLoot): what a gibbed person carried is
                            // thrown out with the chunks instead of destroyed (not robots or animals).
                            if crate::settings::get("FixGibLoot") > 0.5 && !npc.hp_body.robot && !npc.animal {
                                for (k, w) in npc.arsenal.iter().enumerate() {
                                    let a = npc.yaw + k as f64 * 2.4;
                                    weapon_drops.0.push((
                                        w.class.clone(),
                                        center + DVec3::new(a.cos(), a.sin(), 0.0) * (npc.params.radius * 0.8),
                                    ));
                                }
                                npc.arsenal.clear();
                                npc.weapon = None;
                            }
                            takedowns.log.push(Takedown {
                                alliance: npc.alliance.clone(),
                                name: lvl.actors[npc.actor].name.clone(),
                                class: lvl.actors[npc.actor].class.clone(),
                                lethal: true,
                                by_player: h.attacker.is_none(),
                                damage_type: h.damage_type,
                            });
                            if npc.important && !npc.bind_name.is_empty() {
                                flags.0.set_bool(&format!("{}_Dead", npc.bind_name), true);
                            }
                            npc.state = NpcState::Dead;
                            npc.weapon_drawn = false;
                            npc.carried = true; // gone: nothing left to search or carry
                            npc.searched = true;
                            blockers.list[npc.blocker].radius = 0.0;
                            blockers.list[npc.blocker].height = 0.0;
                            deco.body.physics = imsim_core::body::BodyPhysics::None;
                            println!(
                                "PAWN {} {} ({})",
                                lvl.actors[npc.actor].name,
                                if npc.hp_body.robot {
                                    "destroyed"
                                } else {
                                    "gibbed"
                                },
                                h.damage_type
                            );
                            break;
                        }
                        // Animals can't be knocked out: a baton or tranquilizer dart kills them. In water nobody is
                        // only unconscious (DeusExCarcass: "you will die in water every time").
                        let in_water = crate::actors::water_at(lvl, me_center_of(&blockers, npc.blocker)).is_some();
                        let knocked_out = knocked_out && !npc.animal && !in_water;
                        if in_water && !npc.hp_body.robot {
                            if let Some(Value::Path(c)) = find(&lvl.actors[npc.actor].props, "CarcassType") {
                                npc.water_carcass = Some((c.rsplit('.').next().unwrap_or("").to_string(), 1.5));
                            }
                        }
                        npc.state = NpcState::Dead;
                        npc.knocked_out = knocked_out;
                        npc.weapon_drawn = false;
                        // DeusExCarcass.InitFor: the body alerts others only if dead (not unconscious), not a robot.
                        npc.killer_alliance = match h.attacker {
                            None => "Player".to_string(),
                            Some(b) => snap_of(b).map(|p| p.alliance.clone()).unwrap_or_default(),
                        };
                        npc.killer_bind = match h.attacker {
                            None => "JCDenton".to_string(),
                            Some(b) => snap_of(b).map(|p| p.bind.clone()).unwrap_or_default(),
                        };
                        npc.emit_carcass = !knocked_out && !npc.hp_body.robot;
                        npc.alert = None;
                        // ScriptedPawn.Died: important characters leave <BindName>_Dead (and _Unconscious if
                        // knocked out) flags that never expire.
                        if npc.important && !npc.bind_name.is_empty() {
                            flags.0.set_bool(&format!("{}_Dead", npc.bind_name), true);
                            if knocked_out {
                                flags
                                    .0
                                    .set_bool(&format!("{}_Unconscious", npc.bind_name), true);
                            }
                        }
                        takedowns.log.push(Takedown {
                            alliance: npc.alliance.clone(),
                            name: lvl.actors[npc.actor].name.clone(),
                            class: lvl.actors[npc.actor].class.clone(),
                            lethal: !knocked_out,
                            by_player: h.attacker.is_none(),
                            damage_type: h.damage_type,
                        });
                        blockers.list[npc.blocker].radius = 0.0;
                        blockers.list[npc.blocker].height = 0.0;
                        deco.body.physics = imsim_core::body::BodyPhysics::None;
                        let ma = &lvl.mesh_actors[an.mesh_actor];
                        if let Some(anim) = &ma.anim {
                            let name = if npc.sitting {
                                "DeathFront"
                            } else if fall_back {
                                "DeathBack"
                            } else {
                                "DeathFront"
                            };
                            an.clip = Clip::named(&anim.mesh, name, false);
                            an.time = 0.0;
                            // Animal.bPlayDying False (rats, birds...): no death animation; the carcass appears at once.
                            if an.clip.is_none() {
                                if let Some(Value::Path(c)) =
                                    find(&lvl.actors[npc.actor].props, "CarcassType")
                                {
                                    npc.swap_carcass =
                                        Some(c.rsplit('.').next().unwrap_or("").to_string());
                                }
                            }
                        }
                        println!(
                            "PAWN {} {} ({})",
                            lvl.actors[npc.actor].name,
                            if knocked_out { "knocked out" } else { "killed" },
                            h.damage_type
                        );
                        // PlayDyingSound: the Die cry, a LoudNoise and a Distress call (friends only answer it if this
                        // pawn had an enemy it had seen, so a silent takedown from behind stays silent).
                        if let Some(snd) = crate::sound::prop(&lvl.actors[npc.actor].props, "Die") {
                            crate::sound::play_owned(
                                crate::sound::npc_owner(npc.blocker),
                                crate::sound::slot::PAIN,
                                &snd,
                                Some(center),
                                crate::sound::fprop(
                                    &lvl.actors[npc.actor].props,
                                    "TransientSoundVolume",
                                    1.0,
                                ),
                                0.0,
                                crate::sound::random_pitch(),
                            );
                        }
                        let me = Some(Target::Pawn(npc.blocker));
                        new_noises.push(Noise {
                            pos: center,
                            volume: 1.0,
                            radius: 800.0,
                            maker: me,
                            instigator: me,
                            kind: NoiseKind::Loud,
                        });
                        new_noises.push(Noise {
                            pos: center,
                            volume: 1.0,
                            radius: 800.0,
                            maker: me,
                            instigator: me,
                            kind: NoiseKind::Distress,
                        });
                        break;
                    }
                    crate::damage::Outcome::Hurt { hit, pain, stunned } => {
                        // TakeDamage: 'Flamed' sets a pawn alight (CatchFire), robots excepted.
                        if h.damage_type == "Flamed"
                            && npc.on_fire.is_none()
                            && npc.burn_period > 0.0
                            && !npc.hp_body.robot
                        {
                            npc.on_fire = Some(0.0);
                            npc.burn_tick = 0.0;
                            println!("PAWN {} caught fire", lvl.actors[npc.actor].name);
                        }
                        // GMDX Hemorrhage: a player's shuriken (crate::perks) starts the bleed (StartBleed:
                        // 16 ticks of int(Damage x 0.1)); each BleedEffect tick sprays blood drops.
                        if crate::perks::take_bleed(npc.blocker, h.damage) {
                            npc.bleed_counter = 16;
                            npc.bleed_timer = 0.0;
                            npc.bleed_damage = (h.damage * 0.1).floor();
                        }
                        if h.damage_type == "BleedEffect" && npc.can_bleed {
                            for i in 0..10u32 {
                                if (npc.blocker as u32 * 31 + i * 17 + npc.bleed_counter * 7) % 2 == 0 {
                                    drips.0.push(center + DVec3::Z * npc.params.height * 0.3);
                                }
                            }
                        }
                        if h.damage_type == "Poison" {
                            // StartPoison: eight more doses, two seconds apart.
                            npc.poison_counter = 8;
                            npc.poison_timer = 0.0;
                            npc.poison_damage = npc.poison_damage.max(h.damage);
                            npc.poisoner = h.attacker;
                        }
                        println!(
                            "PAWN {} hurt {:?} pain={} stunned={} health={:.0} parts={:?}",
                            lvl.actors[npc.actor].name,
                            hit,
                            pain,
                            stunned,
                            npc.hp_body.health,
                            npc.hp_body.parts
                        );
                        // PlayTakeHitSound: HitSound1 up to 30 damage, HitSound2 above, at most every 0.25 s; the cry
                        // of pain is a Distress call.
                        {
                            let props = &lvl.actors[npc.actor].props;
                            let which = if h.damage <= 30.0 {
                                "HitSound1"
                            } else {
                                "HitSound2"
                            };
                            if let Some(snd) = crate::sound::prop(props, which) {
                                let vol = crate::sound::fprop(props, "TransientSoundVolume", 1.0)
                                    .max(2.0);
                                crate::sound::play_req(crate::sound::Req {
                                    sound: snd,
                                    pos: Some(center),
                                    volume: vol,
                                    radius: 0.0,
                                    pitch: crate::sound::random_pitch(),
                                    owner: crate::sound::npc_owner(npc.blocker),
                                    slot: crate::sound::slot::PAIN,
                                    no_override: false,
                                    min_gap: 0.25,
                                });
                            }
                        }
                        let me = Some(Target::Pawn(npc.blocker));
                        new_noises.push(Noise {
                            pos: center,
                            volume: 1.0,
                            radius: 800.0,
                            maker: me,
                            instigator: me,
                            kind: NoiseKind::Distress,
                        });
                        // RubbingEyes: BlockReactions - no ReactToInjury (no new enemy, no fear, no pain animation),
                        // and TakeDamageBase without bPlayAnim.
                        if !matches!(npc.state, NpcState::RubbingEyes(_)) {
                            // ReactToInjury: IncreaseAgitation toward the attacker's alliance (members of the own alliance
                            // never count), then SetEnemy only succeeds if that alliance is now hostile. Friendly fire just
                            // hurts.
                            let (who, alliance) = match h.attacker {
                                None => (Target::Player, "Player".to_string()),
                                Some(b) => (
                                    Target::Pawn(b),
                                    snap_of(b).map(|p| p.alliance.clone()).unwrap_or_default(),
                                ),
                            };
                            // ShouldReactToInjuryType: bHate/bFearInjury for direct harm, bHate/bFearIndirectInjury for
                            // gas, blasts and the like; fear of being hurt is IncreaseFear(instigator, 2.0).
                            let hate_this = imsim_core::reactions::reacts_to_injury(
                                h.damage_type,
                                npc.hate_injury,
                                npc.hate_indirect_injury,
                            );
                            let fear_this = imsim_core::reactions::reacts_to_injury(
                                h.damage_type,
                                npc.fear_injury,
                                npc.fear_indirect_injury,
                            );
                            if hate_this {
                                npc.alliances.provoke(&alliance, None);
                            }
                            if fear_this {
                                npc.fear.increase(2.0, 0.0);
                            }
                            let made_enemy = npc.alliances.hostile_to(&alliance)
                                || (who != Target::Player && snap_of_hostile(npc, who));
                            if made_enemy {
                                npc.target = Some(who);
                                npc.last_pos = target_pos(who, player_center, &blockers.list);
                                npc.last_seen = now;
                                npc.route = None;
                            } else if fear_this && npc.fear.fearful() {
                                // SetEnemy(instigatedBy, , true); SetNextState('Fleeing'): runs once the hit plays out.
                                npc.target = Some(who);
                                npc.last_pos = target_pos(who, player_center, &blockers.list);
                                npc.last_seen = now;
                                npc.fear_flee = true;
                                if !matches!(npc.state, NpcState::Fleeing) {
                                    npc.state = NpcState::Fleeing;
                                    npc.route = None;
                                    npc.flee_dest = None;
                                }
                            }
                            if npc.sitting && (stunned || pain) {
                                npc.sitting = false;
                                npc.seat = None;
                                if let Some(b) = npc.body.as_mut() {
                                    b.physics = Physics::Falling;
                                }
                            }
                            if stunned && !npc.hp_body.robot {
                                npc.state = NpcState::Stunned(15.0);
                                npc.weapon_drawn = false;
                                npc.anim = "";
                            } else if pain {
                                if let Some(name) = hit.anim() {
                                    let ma = &lvl.mesh_actors[an.mesh_actor];
                                    if let Some(c) = ma
                                        .anim
                                        .as_ref()
                                        .and_then(|a| Clip::named(&a.mesh, name, false))
                                    {
                                        let dur =
                                            (c.frames as f32 - 1.0).max(1.0) / c.rate.max(1.0) + 0.1;
                                        an.clip = Some(c);
                                        npc.anim = name;
                                        npc.state = NpcState::TakingHit(dur);
                                        // GMDX Piercing: 1.3 s with a hand-to-hand weapon in the player's hand
                                        // (Grays and Karkians are slower to flinch there, so they keep 2).
                                        let tough = matches!(
                                            lvl.actors[npc.actor].class.as_str(),
                                            "Gray" | "Karkian" | "KarkianBaby"
                                        );
                                        npc.take_hit_timer = crate::perks::flinch_time()
                                            .filter(|_| !tough)
                                            .unwrap_or(2.0);
                                    }
                                }
                            } else if matches!(npc.state, NpcState::Idle | NpcState::Seeking)
                                && made_enemy
                            {
                                let (t, pos, seen) = (
                                    npc.target.unwrap_or(Target::Player),
                                    npc.last_pos,
                                    npc.last_seen,
                                );
                                npc.begin_attack(t, pos, seen);
                            }
                        }
                    }
                }
            }
            if npc.state == NpcState::Dead {
                continue;
            }
        }

        let me = blockers.list[npc.blocker];
        let my_eye = eye_of(&me);
        // LastRenderTime: on screen = inside the view (a cone a little wider than the 90 degree view, so the
        // corners count) and some part of the cylinder not hidden by the world from the camera.
        npc.render_check -= dt;
        if npc.render_check <= 0.0 {
            npc.render_check = 0.2;
            let eye = _player_eye;
            let fwd = {
                let f = cam.forward().as_vec3();
                DVec3::new(f.x as f64, f.z as f64, f.y as f64)
            };
            let to = me.center - eye;
            let dist = to.length();
            let in_view = dist < me.radius + 60.0 || to.dot(fwd) / dist.max(1e-6) > 0.45;
            if in_view
                && [me.height * 0.8, 0.0, -me.height * 0.8].iter().any(|&dz| {
                    lvl.collision.trace(eye, me.center + DVec3::new(0.0, 0.0, dz), DVec3::ZERO).fraction >= 0.999
                })
            {
                npc.last_rendered = now;
            }
        }
        let dist_player = (me.center - player_center).length();
        let check_other = !(npc.tick_visible && dist_player > 600.0 && now - npc.last_rendered >= 5.0);
        let check_player = !(npc.tick_visible && dist_player > 2500.0);
        let world = World {
            level: lvl,
            blockers: &blockers.list,
            ignore: Some(npc.blocker),
            pushable: None,
            movers: Some(&mv.rt),
        };
        npc.fire_timer = (npc.fire_timer - dt).max(0.0);
        if npc.crazed > 0.0 {
            npc.crazed = (npc.crazed - dt).max(0.0);
            if npc.crazed == 0.0 {
                npc.target = None;
                // GMDX Sensor Burnout (Robot.StartReboot): the scrambled bot shuts down for PerkValue seconds
                // before it goes back to its orders.
                if let Some(v) = crate::perks::value_if("PerkSensorBurnout") {
                    if npc.hp_body.robot && npc.state != NpcState::Dead {
                        npc.state = NpcState::Stunned(v);
                        npc.anim = "";
                    }
                }
            }
        }
        // Gray.Tick: every damageInterval, radiation to the player if visible within damageRadius (hostile or not).
        if let Some((radius, interval, amount)) = npc.aura {
            npc.aura_timer += dt;
            if npc.aura_timer >= interval {
                npc.aura_timer = 0.0;
                if !ph.dead
                    && (player_center - me.center).length() <= radius
                    && world
                        .trace(me.center, player_center, DVec3::splat(0.5))
                        .fraction
                        >= 0.999
                {
                    ph.damage(amount, "Radiation", DVec3::ZERO);
                }
            }
        }

        // Senses. ReactionLevel climbs while anything is up (UpdateReactionLevel) and the surprise pause on
        // attacking is what it hasn't covered yet; it ebbs over 7 seconds otherwise.
        let stunned_now = matches!(
            npc.state,
            NpcState::Stunned(_) | NpcState::TakingHit(_) | NpcState::RubbingEyes(_)
        );
        if npc.readiness > 0.0 || npc.state == NpcState::Seeking {
            npc.reaction = (npc.reaction + dt / npc.surprise.max(1e-6)).min(1.0);
        } else {
            npc.reaction = (npc.reaction - dt / 7.0).max(0.0);
        }
        let facing = DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0);
        let (sight_radius, fov_cos) = (npc.sight_radius, npc.fov_cos);
        // AICanSee: within SightRadius, inside the field of view (when checked), and a clear line from the eyes
        // to the head, middle or feet. Returns the distance.
        let sees =
            |center: DVec3, height: f64, check_dir: bool, other: Option<usize>| -> Option<f64> {
                let dist = (center - my_eye).length();
                if dist > sight_radius {
                    return None;
                }
                let flat =
                    DVec3::new(center.x - my_eye.x, center.y - my_eye.y, 0.0).normalize_or_zero();
                if check_dir && dist > me.radius + 8.0 && flat.dot(facing) < fov_cos {
                    return None;
                }
                let clear = [height * 0.8, 0.0, -height * 0.8].iter().any(|&dz| {
                    let h =
                        world.trace(my_eye, center + DVec3::new(0.0, 0.0, dz), DVec3::splat(1.0));
                    h.fraction >= 0.999 || (other.is_some() && h.actor == other)
                });
                clear.then_some(dist)
            };
        let valid_enemy = |npc: &Npc, t: Target| -> bool {
            match t {
                Target::Player => !ph.dead && npc.hates("Player"),
                Target::Pawn(b) => {
                    b != npc.blocker
                        && snap_of(b).map_or(false, |p| !p.dead)
                        && snap_of_hostile(npc, t)
                }
            }
        };
        npc.alliances.tick(dt);
        npc.cycle += dt;
        npc.think -= dt;
        if npc.think <= 0.0 && npc.weapon.is_some() && !stunned_now {
            npc.think = 0.1;
            let period = std::mem::take(&mut npc.cycle);
            if matches!(npc.state, NpcState::Idle | NpcState::Seeking) && npc.react_presence {
                // CheckEnemyPresence: the nearest enemy in view. The player counts for how visible they are
                // (light and movement, faded with distance, against VisibilityThreshold); other pawns are seen at once.
                let mut cand: Option<(Target, DVec3, f64, f32)> = None;
                // bHateCarcass: the alliance that killed a friend is hated on sight for 15 s (PotentialEnemyAlliance).
                let potential_player = npc
                    .potential_enemy
                    .as_ref()
                    .is_some_and(|(a, _)| a == "Player")
                    && !ph.dead;
                if check_player
                    && (valid_enemy(npc, Target::Player) || potential_player)
                    && !crate::abilities::invisible_to(npc.hp_body.robot, ph.camo)
                {
                    if let Some(d) = sees(player_center, player.params.height, true, None) {
                        if potential_player && !npc.hates("Player") {
                            npc.alliances.agitate("Player", 1.0);
                            npc.potential_enemy = None;
                        }
                        let vis = crate::stealth::sight_visibility(
                            crate::stealth::player_visibility(&plight, player.body.velocity),
                            d,
                            npc.sight_radius,
                            npc.vis_threshold,
                        );
                        if vis > 0.0 {
                            cand =
                                Some((Target::Player, player_center, d, vis + npc.vis_threshold));
                        }
                    }
                }
                for p in pawns.iter().filter(|_| check_other) {
                    let potential = !p.dead
                        && npc
                            .potential_enemy
                            .as_ref()
                            .is_some_and(|(a, _)| a.eq_ignore_ascii_case(&p.alliance));
                    if !valid_enemy(npc, Target::Pawn(p.blocker)) && !potential {
                        continue;
                    }
                    if potential
                        && sees(
                            blockers.list[p.blocker].center,
                            blockers.list[p.blocker].height,
                            true,
                            Some(p.blocker),
                        )
                        .is_some()
                    {
                        npc.alliances.agitate(&p.alliance.clone(), 1.0);
                        npc.potential_enemy = None;
                    }
                    if !valid_enemy(npc, Target::Pawn(p.blocker)) {
                        continue;
                    }
                    let c = blockers.list[p.blocker];
                    let d = (c.center - my_eye).length();
                    if cand.map_or(true, |(_, _, bd, _)| d < bd)
                        && sees(c.center, c.height, true, Some(p.blocker)).is_some()
                    {
                        cand = Some((Target::Pawn(p.blocker), c.center, d, 100000.0));
                    }
                }
                // CheckCycle: readiness climbs at up to 2/s (full view) or as slow as 1/4.5 s (barely visible), holds
                // for 3 s after losing sight, then decays over 4 s.
                match cand {
                    None => {
                        npc.sustain -= period;
                        if npc.sustain <= 0.0 {
                            npc.sustain = 0.0;
                            npc.readiness = (npc.readiness - period / 4.0).max(0.0);
                        }
                    }
                    Some((t, pos, _, cum)) => {
                        npc.sustain = 3.0;
                        let c = (cum * 2.0).clamp(0.5 / 4.5, 1.0);
                        npc.readiness += c * period / 0.5;
                        if npc.readiness >= 1.0 {
                            npc.begin_attack(t, pos, now);
                        } else if npc.readiness >= npc.sight_pct {
                            // HandleSighting.
                            if npc.state == NpcState::Idle {
                                npc.set_seek_location(pos, SeekKind::Sight, false);
                                npc.enter_seeking();
                            } else if now - npc.last_seen > 2.0 {
                                npc.reseek(pos, SeekKind::Sight);
                            }
                        }
                    }
                }
            } else if matches!(
                npc.state,
                NpcState::Attacking | NpcState::Surprised(_) | NpcState::Fleeing
            ) {
                // UpdateActorVisibility in combat: lighting no longer matters, nor which way we face.
                if let Some(t) = npc.target {
                    let (c, h, other) = match t {
                        Target::Player => (player_center, player.params.height, None),
                        Target::Pawn(b) => {
                            (blockers.list[b].center, blockers.list[b].height, Some(b))
                        }
                    };
                    if (valid_enemy(npc, t) || (npc.state == NpcState::Fleeing && npc.fear_flee))
                        && !(t == Target::Player
                            && crate::abilities::invisible_to(npc.hp_body.robot, ph.camo))
                        && sees(c, h, false, other).is_some()
                    {
                        npc.last_seen = now;
                        npc.last_pos = c;
                    }
                }
            }
        }

        // The reaction callbacks (ScriptedPawn.UpdateReactionCallbacks): which stimuli this state listens for
        // (SetReactions), and what the pawn's bReact / bHate / bFear flags make of each.
        npc.fear.tick(dt);
        npc.futz_timer = (npc.futz_timer - dt).max(0.0);
        let lf = npc.looking_for(con_look.is_some(), !crate::consys::third_person());
        let alliance_of = |t: Target| -> String {
            match t {
                Target::Player => "Player".to_string(),
                Target::Pawn(b) => snap_of(b).map(|p| p.alliance.clone()).unwrap_or_default(),
            }
        };
        // HandleEnemy: HandlingEnemy -> Attacking, which runs instead for a pawn with nothing to fight with.
        macro_rules! handle_enemy {
            ($t:expr, $pos:expr, $seen:expr) => {{
                let (t, pos, seen) = ($t, $pos, $seen);
                if npc.weapon.is_some() {
                    npc.begin_attack(t, pos, seen);
                } else {
                    npc.flee_from(t, pos, now, false);
                }
            }};
        }
        if !stunned_now {
            for n in &heard {
                if n.maker == Some(Target::Pawn(npc.blocker))
                    || crate::stealth::hear(lvl, my_eye, n) < npc.hear_threshold
                {
                    continue;
                }
                // AISetEventCallback('WeaponFire' / 'Distress', ..., bCheckVisibility, bCheckDir, , bCheckLOS): a shot or a
                // cry for help only counts if this pawn can see where it came from and is facing that way. Anything
                // else heard through walls is a LoudNoise, which only sends it to look.
                if matches!(n.kind, NoiseKind::WeaponFire | NoiseKind::Distress) {
                    let other = match n.maker {
                        Some(Target::Pawn(b)) => Some(b),
                        _ => None,
                    };
                    if sees(n.pos, 40.0, true, other).is_none() {
                        continue;
                    }
                }
                match n.kind {
                    NoiseKind::WeaponFire => {
                        // HandleShot: React, Fear, Hate.
                        if !(lf.shot && (npc.react_shot || npc.fear_shot || npc.hate_shot)) {
                            continue;
                        }
                        let Some(t) = n.instigator else { continue };
                        if t == Target::Pawn(npc.blocker) {
                            continue;
                        }
                        let alliance = alliance_of(t);
                        if npc.hate_shot {
                            npc.alliances.provoke(&alliance, None);
                        }
                        if npc.fear_shot {
                            npc.fear.increase(1.0, 0.0);
                        }
                        let pos = target_pos(t, player_center, &blockers.list);
                        if valid_enemy(npc, t) {
                            handle_enemy!(t, pos, now);
                        } else if npc.fear_shot && npc.fear.fearful() {
                            npc.flee_from(t, pos, now, true);
                        } else if t == Target::Player {
                            npc.react_to_futz();
                        }
                    }
                    NoiseKind::Loud => {
                        // HandleLoudNoise (LoudNoiseScore culls anything not made by an enemy): go and look.
                        if !(lf.loud_noise && npc.react_loud) {
                            continue;
                        }
                        if let Some(t) = n.instigator {
                            if valid_enemy(npc, t) && npc.weapon.is_some() {
                                if npc.state == NpcState::Idle {
                                    npc.set_seek_location(n.pos, SeekKind::Sound, false);
                                    npc.enter_seeking();
                                } else if now - npc.last_seen > 2.0 {
                                    npc.reseek(n.pos, SeekKind::Sound);
                                }
                            }
                        }
                    }
                    NoiseKind::Alert => {
                        if !(lf.loud_noise && npc.react_loud) || npc.weapon.is_none() {
                            continue;
                        }
                        if n.maker == Some(Target::Pawn(npc.blocker)) {
                            continue;
                        }
                        if npc.state == NpcState::Idle {
                            npc.set_seek_location(n.pos, SeekKind::Sound, false);
                            npc.enter_seeking();
                        } else if npc.state == NpcState::Seeking && now - npc.last_seen > 2.0 {
                            npc.reseek(n.pos, SeekKind::Sound);
                        }
                    }
                    NoiseKind::Distress => {
                        // HandleDistress: a friend in trouble who has seen their attacker.
                        if !(lf.distress && (npc.hate_distress || npc.react_distress || npc.fear_distress)) {
                            continue;
                        }
                        let Some(Target::Pawn(d)) = n.maker else {
                            continue;
                        };
                        let Some(victim) = snap_of(d) else { continue };
                        let Some(foe) = victim.target else { continue };
                        if foe == Target::Pawn(npc.blocker) {
                            continue;
                        }
                        // DistressScore: only friends' cries count (anyone's for bFearDistress).
                        let friend = npc.friendly(&victim.alliance);
                        if !friend && !npc.fear_distress {
                            continue;
                        }
                        if npc.fear_distress {
                            npc.fear.increase(1.0, 0.0);
                        }
                        let foe_pos = target_pos(foe, player_center, &blockers.list);
                        let mut attacking = false;
                        if friend && now - victim.last_seen <= victim.enemy_timeout {
                            if npc.hate_distress {
                                npc.alliances.provoke(&alliance_of(foe), Some(1.0));
                            }
                            if valid_enemy(npc, foe) && (npc.react_distress || npc.hate_distress) {
                                handle_enemy!(foe, foe_pos, victim.last_seen);
                                attacking = true;
                            }
                        }
                        if !attacking && npc.fear_distress && npc.fear.fearful() {
                            npc.flee_from(foe, foe_pos, now, true);
                        }
                    }
                }
            }
        }
        // Continuous sights, checked every 0.25 s like a pulsing AIStartEvent: the player with a weapon out
        // (HandleWeapon), the player picking a lock or bypassing a device (HandleHacking), and one-off futzing
        // with owned things (HandleFutz).
        npc.stim_check -= dt;
        if npc.stim_check <= 0.0 && !stunned_now && !ph.dead {
            npc.stim_check = 0.25;
            let seen_player = || sees(player_center, player.params.height, true, None).is_some();
            if stim.weapon_drawn
                && lf.weapon
                && (npc.hate_weapon || npc.fear_weapon)
                && !valid_enemy(npc, Target::Player)
                && seen_player()
            {
                if npc.hate_weapon {
                    npc.alliances.provoke("Player", None);
                }
                if npc.fear_weapon {
                    npc.fear.increase(1.0, 0.0);
                }
                // Presence checking takes it from here if that made the player an enemy.
                if !valid_enemy(npc, Target::Player) {
                    if npc.fear_weapon && npc.fear.fearful() {
                        npc.flee_from(Target::Player, player_center, now, true);
                    } else {
                        npc.react_to_futz();
                    }
                }
            }
            if let Some(at) = stim.hacking {
                if lf.hacking && (npc.hate_hacking || npc.fear_hacking) && sees(at, 24.0, true, None).is_some() {
                    if npc.hate_hacking {
                        npc.alliances.provoke("Player", Some(1.0));
                    }
                    if npc.fear_hacking {
                        npc.fear.increase(0.51, 0.0);
                    }
                    if valid_enemy(npc, Target::Player) {
                        handle_enemy!(Target::Player, player_center, now);
                    } else if npc.fear_hacking && npc.fear.fearful() {
                        npc.flee_from(Target::Player, player_center, now, true);
                    } else {
                        npc.react_to_futz();
                    }
                }
            }
        }
        if lf.futz && npc.react_futz && !stunned_now {
            for at in &stim.futz {
                if sees(*at, 16.0, true, None).is_some() {
                    npc.react_to_futz();
                }
            }
        }

        npc.alarm_timer = (npc.alarm_timer - dt).max(0.0);
        if let Some((_, t)) = npc.potential_enemy.as_mut() {
            *t -= dt;
        }
        if npc.potential_enemy.as_ref().is_some_and(|(_, t)| *t <= 0.0) {
            npc.potential_enemy = None;
        }
        let looking = matches!(npc.state, NpcState::Idle | NpcState::Seeking) && !stunned_now;
        // HandleAlarm: an alarm going off within earshot sends pawns who hate whoever set it off to search there.
        // The klaxon (AIStartEvent('Alarm', EAITYPE_Audio, 128/255, 25 x 65)) carries its full volume out to the
        // radius: the lobby guards hear the statue's front camera and run out to where it saw the player.
        if looking && lf.alarm && (npc.react_alarm || npc.fear_alarm) && npc.alarm_timer <= 0.0 {
            for u in alarms.units.iter().filter(|u| u.active) {
                let d = u.pos.distance(my_eye);
                let r = crate::alarms::AlarmRt::HEAR_RADIUS;
                if d >= r || (128.0 / 255.0) < npc.hear_threshold {
                    continue;
                }
                // HandleAlarm, bFearAlarm: IncreaseFear(instigator, 2.0) -> flee.
                if npc.fear_alarm {
                    npc.fear.increase(2.0, 0.0);
                    if npc.fear.fearful() {
                        let t = u.instigator.unwrap_or(Target::Player);
                        npc.flee_from(t, u.seek, now, true);
                        break;
                    }
                }
                if let Some(t) = u.instigator {
                    if valid_enemy(npc, t) && npc.weapon.is_some() {
                        npc.alarm_timer = 120.0;
                        if npc.state == NpcState::Idle {
                            npc.set_seek_location(u.seek, SeekKind::Sound, false);
                            npc.enter_seeking();
                        } else {
                            npc.reseek(u.seek, SeekKind::Sound);
                        }
                        break;
                    }
                }
            }
        }
        // CheckCarcassPresence: a body within 1200 that this pawn cares about (a friend killed by someone it hates,
        // or any friend for bHateCarcass) and can see: search there; bHateCarcass also makes the killers' alliance
        // an enemy on sight for 15 s. Unconscious bodies don't count (bEmitCarcass).
        npc.carcass_check -= dt;
        if looking
            && (npc.hate_carcass || npc.react_carcass || npc.fear_carcass)
            && npc.carcasses_seen.len() < 4
            && npc.carcass_check <= 0.0
        {
            npc.carcass_check = 0.1;
            let body_alarms = body_alarms();
            let found = pawns.iter().find(|p| {
                p.dead
                    && (p.emit_carcass || body_alarms && p.any_body)
                    && p.on_ground
                    && p.blocker != npc.blocker
                    && !npc.carcasses_seen.contains(&p.blocker)
                    && p.pos.distance(me.center) < 1200.0
                    && (npc.fear_carcass
                        || (npc.alliances.friendly_to(&p.alliance)
                            && (npc.hate_carcass
                                || (npc.react_carcass && npc.hates(&p.killer_alliance)))))
                    && sees(p.pos, 8.0, true, None).is_some()
            });
            if let Some(p) = found {
                npc.carcasses_seen.push(p.blocker);
                if npc.hate_carcass && !p.killer_alliance.is_empty() {
                    npc.potential_enemy = Some((p.killer_alliance.clone(), 15.0));
                }
                if npc.state == NpcState::Idle {
                    npc.set_seek_location(p.pos, SeekKind::Carcass, false);
                    npc.enter_seeking();
                } else {
                    npc.reseek(p.pos, SeekKind::Carcass);
                }
                // Enhancement (Bodies Raise Alarms): an armed pawn who finds a friend's body runs for the nearest
                // alarm (then searches the body), or else shouts so others nearby come and look.
                if body_alarms && npc.weapon.is_some() && npc.alarm_timer <= 0.0 && npc.alliances.friendly_to(&p.alliance) {
                    let me_b = npc.blocker;
                    let al = npc.alliances.clone();
                    if let Some(i) = alarms.find_target(me_b, me.center, |a| al.hostile_to(a) || a == "Player") {
                        alarms.units[i].associated = Some(me_b);
                        npc.alert = Some((i, 0.0));
                        npc.state = NpcState::Alerting;
                        npc.route = None;
                        npc.body_alarm = Some(p.pos);
                        println!("PAWN {} runs for the alarm (a body)", lvl.actors[npc.actor].name);
                    } else {
                        npc.alarm_timer = 30.0;
                        let me = Some(Target::Pawn(npc.blocker));
                        new_noises.push(Noise { pos: me_center_of(&blockers, npc.blocker), volume: 1.0, radius: 1200.0, maker: me, instigator: None, kind: NoiseKind::Alert });
                    }
                }
                println!(
                    "PAWN {} found a body (killed by {})",
                    lvl.actors[npc.actor].name, p.killer_alliance
                );
            }
        }
        // ReactToProjectiles: a live grenade in view. If whoever threw it is an enemy, fight them; otherwise
        // (bAvoidHarm) get away from it (AvoidingProjectiles).
        if (npc.react_projectiles || npc.avoid_harm)
            && !stunned_now
            && !matches!(npc.state, NpcState::Dead | NpcState::Burning)
        {
            for (pos, blast, from) in &danger {
                let near = pos.distance(me.center) < blast.max(160.0) * 1.5;
                if !near || sees(*pos, 4.0, true, None).is_none() {
                    continue;
                }
                let who = match from {
                    None => Target::Player,
                    Some(b) => Target::Pawn(*b),
                };
                if npc.react_projectiles
                    && valid_enemy(npc, who)
                    && matches!(npc.state, NpcState::Idle | NpcState::Seeking)
                    && npc.weapon.is_some()
                {
                    npc.begin_attack(who, target_pos(who, player_center, &blockers.list), now);
                }
                if npc.avoid_harm && npc.avoiding <= 0.0 {
                    npc.avoiding = 2.5;
                    let away = (me.center - *pos).truncate().normalize_or_zero();
                    let away = if away.length_squared() < 1e-6 {
                        DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0)
                    } else {
                        DVec3::new(away.x, away.y, 0.0)
                    };
                    npc.flee_dest = Some(me.center + away * (blast.max(160.0) * 1.6 + 64.0));
                    npc.route = None;
                }
                break;
            }
        }

        if npc.cleaner && npc.state != NpcState::Dead {
            if let Ok(mut c) = CLEANERS.lock() {
                c.push((me.center, npc.params.radius, npc.params.height));
            }
        }
        // Animal.Wandering.Tick / FrightenedByPawn: every 0.5 s, look for a big pawn within 500 heading this way
        // (ShouldBeStartled) and keep out of its path (AvoidingPawn: ~100 units away from where it's going).
        if npc.flee_big && !stunned_now && matches!(npc.state, NpcState::Idle | NpcState::Seeking) {
            npc.startle_timer += dt;
            if npc.startle_timer > 0.5 {
                npc.startle_timer = 0.0;
                let rat = lvl.actors[npc.actor].class == "Rat";
                let (t_max, k) = if rat {
                    (3.0, 1.5)
                } else if npc.cleaner {
                    (2.0, 0.8)
                } else {
                    (2.0, 0.6)
                };
                let startled = |pos: DVec3, vel: DVec3| -> bool {
                    let speed = vel.length();
                    if speed < 20.0 {
                        return false;
                    }
                    let t = pos.distance(me.center) / speed;
                    t <= t_max && me.center.distance(pos + vel * t) < speed * k
                };
                // Only pawns too tall to step over it count (MaxStepHeight vs 1.5 x its height).
                let mut fear: Option<(DVec3, DVec3)> = None;
                if !ph.dead && startled(player_center, player.body.velocity) {
                    fear = Some((player_center, player.body.velocity));
                }
                if fear.is_none() {
                    for p in &pawns {
                        if p.blocker == npc.blocker
                            || p.dead
                            || p.alliance == npc.alliance
                            || p.height * 0.5 <= me.height
                        {
                            continue;
                        }
                        if p.pos.distance(me.center) < 500.0 && startled(p.pos, p.vel) {
                            fear = Some((p.pos, p.vel));
                            break;
                        }
                    }
                }
                if let Some((pos, vel)) = fear {
                    let speed = vel.length();
                    let t = if speed > 0.0 {
                        pos.distance(me.center) / speed
                    } else {
                        1.0
                    };
                    let ahead = pos + vel * (t * 0.98);
                    // AvoidingPawn.PickDestination: ~100 units (x 0.9 - 1.1) away from where it will be, else away
                    // from where it is, else along its path, else against it (AIDirectionReachable, 20 at least);
                    // else stay put.
                    let mag = 100.0 * (crate::sound::frand() as f64 * 0.2 + 0.9);
                    let along = if speed > 0.0 { vel } else { DVec3::ZERO };
                    let dest = [me.center - ahead, me.center - pos, along, -along]
                        .into_iter()
                        .find_map(|d| direction_reachable_v(lvl, me.center, &npc.params, d, 20.0, mag));
                    npc.flee_dest = Some(dest.unwrap_or(me.center));
                    npc.avoiding = 0.6;
                    npc.route = None;
                }
            }
        }
        if lvl.actors[npc.actor].class == "Rat" && npc.state != NpcState::Dead {
            npc.squeak_timer += dt;
            if npc.squeak_timer > 1.0 {
                npc.squeak_timer = 0.0;
                if crate::sound::frand() < 0.05 {
                    crate::sound::play(
                        "DeusExSounds.Animal.RatSqueak2",
                        me.center,
                        1.0,
                        25.0 * 10.0,
                        1.0,
                    );
                }
            }
        }

        // Bleeding (ScriptedPawn.Tick): drops every ((1 - rate) + 0.1) / clamp(speed / 512, 0.05, 1) seconds.
        if npc.bleed_rate > 0.0 && npc.can_bleed {
            let speed = npc.body.as_ref().map_or(0.0, |b| b.velocity.length()) as f32;
            let period =
                ((1.0 - npc.bleed_rate.clamp(0.0, 1.0)) + 0.1) / (speed / 512.0).clamp(0.05, 1.0);
            npc.drop_counter += dt;
            while npc.drop_counter >= period {
                npc.drop_counter -= period;
                drips.0.push(me.center + DVec3::Z * npc.params.height * 0.5);
            }
            npc.bleed_rate -= dt / npc.clot_period.max(0.1);
            if npc.bleed_rate <= 0.0 {
                npc.bleed_rate = 0.0;
                npc.drop_counter = 0.0;
            }
        }

        // Burning (ScriptedPawn.Tick / Timer UpdateFire): 5 off the torso every second for BurnPeriod; when that
        // kills, a final 10 'Burned'. While alight the pawn is in state Burning.
        if let Some(t) = npc.on_fire {
            let t = t + dt;
            npc.burn_tick += dt;
            if npc.burn_tick >= 1.0 {
                npc.burn_tick -= 1.0;
                npc.hp_body.parts[1] -= 5.0;
                npc.hp_body.total();
                hp.0.insert(npc.blocker, npc.hp_body.health);
                if npc.hp_body.health <= 0.0 {
                    new_hits.push(crate::damage::PawnHit {
                        blocker: npc.blocker,
                        damage: 10.0,
                        damage_type: "Burned",
                        hit_loc: me.center,
                        attacker: None,
                        attacker_pos: me.center,
                    });
                    npc.on_fire = None;
                }
            }
            if npc.on_fire.is_some() {
                npc.on_fire = if t >= npc.burn_period { None } else { Some(t) };
            }
            // ZoneChange into water (bCanSwim): ExtinguishFire.
            if npc.on_fire.is_some() && npc.can_swim && crate::actors::water_at(lvl, me.center).is_some() {
                npc.on_fire = None;
                println!("PAWN {} put out in the water", lvl.actors[npc.actor].name);
            }
            if npc.on_fire.is_some()
                && !matches!(
                    npc.state,
                    NpcState::Burning
                        | NpcState::TakingHit(_)
                        | NpcState::RubbingEyes(_)
                        | NpcState::Dead
                )
            {
                npc.state = NpcState::Burning;
                npc.route = None;
                npc.flee_dest = None;
                npc.weapon_drawn = false;
                npc.seek = None;
            }
        }
        // A third-person conversation blocks reactions (BlockReactions).
        if con_hold
            && npc.state != NpcState::Dead
            && npc.state != NpcState::Idle
            && npc.on_fire.is_none()
        {
            npc.state = NpcState::Idle;
            npc.readiness = 0.0;
            npc.target = None;
        }
        // Act.
        let mut wish = DVec3::ZERO;
        let mut running = false;
        let mut firing = false;
        // Attacking's state code starts over at Begin (RunToRange's TurnToward) each time the pawn attacks anew.
        if npc.state != NpcState::Attacking {
            npc.atk_phase = AtkPhase::TurnFirst;
        }
        match npc.state {
            NpcState::Dead => {}
            NpcState::TakingHit(t) => {
                let t = t - dt;
                npc.state = if t > 0.0 {
                    NpcState::TakingHit(t)
                } else if npc.target.is_some() && npc.fear_flee {
                    NpcState::Fleeing
                } else if npc.target.is_some() {
                    NpcState::Attacking
                } else {
                    NpcState::Idle
                };
                if t <= 0.0 {
                    npc.anim = "";
                }
            }
            NpcState::RubbingEyes(t) => {
                npc.weapon_drawn = false;
                let t = t - dt;
                if t > 0.0 {
                    npc.state = NpcState::RubbingEyes(t);
                } else {
                    // RubEyesStop, then back to whatever comes next (the fight, usually).
                    let ma = &lvl.mesh_actors[an.mesh_actor];
                    let stop = ma
                        .anim
                        .as_ref()
                        .and_then(|a| Clip::named(&a.mesh, "RubEyesStop", false));
                    let len = stop
                        .as_ref()
                        .map(|c| (c.frames as f32 - 1.0).max(1.0) / c.rate.max(1.0) + 0.1)
                        .unwrap_or(0.1);
                    if let Some(c) = stop {
                        an.clip = Some(c);
                        npc.anim = "RubEyesStop";
                    }
                    npc.last_seen = now;
                    npc.state = NpcState::TakingHit(len);
                }
            }
            NpcState::Stunned(t) => {
                npc.weapon_drawn = false;
                let t = t - dt;
                if t <= 0.0 {
                    npc.state = if npc.target.is_some() {
                        NpcState::Surprised(0.3)
                    } else {
                        NpcState::Idle
                    };
                    npc.last_seen = now;
                    npc.anim = "";
                } else {
                    npc.state = NpcState::Stunned(t);
                }
            }
            NpcState::Idle => {
                if let Some(look) = con_look {
                    // Conversation / FirstPersonConversation: stand still, look at whoever's talking.
                    npc.route = None;
                    if let Some(l) = look {
                        let to = l - me.center;
                        npc.yaw = turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64);
                    }
                } else {
                    let sees = npc.orders == Orders::WaitingFor
                        && lvl
                            .collision
                            .trace(my_eye, player_center, DVec3::ZERO)
                            .fraction
                            >= 1.0;
                    let (w, run) = do_orders(lvl, npc, me.center, dt, player_center, sees);
                    wish = w;
                    running = run;
                }
            }
            NpcState::Seeking => {
                npc.weapon_drawn = true;
                let (w, run) = do_seek(lvl, npc, &me, my_eye, &world, dt, now);
                wish = w;
                running = run;
            }
            NpcState::Surprised(t) => {
                // Stand up from a chair, draw the weapon and turn to face the threat.
                if npc.sitting {
                    npc.sitting = false;
                    npc.seat = None;
                    if let Some(b) = npc.body.as_mut() {
                        b.physics = Physics::Falling;
                    }
                }
                npc.weapon_drawn = true;
                npc.route = None;
                npc.pause = 0.0;
                let to = npc.last_pos - me.center;
                npc.yaw = turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64);
                let t = t - dt;
                npc.state = if t <= 0.0 {
                    NpcState::Attacking
                } else {
                    NpcState::Surprised(t)
                };
            }
            NpcState::Burning => {
                npc.weapon_drawn = false;
                if npc.on_fire.is_none() {
                    npc.state = if npc.target.is_some() {
                        NpcState::Attacking
                    } else {
                        NpcState::Idle
                    };
                    npc.route = None;
                    npc.flee_dest = None;
                } else {
                    // PickDestination: about 300 units off, away from the enemy if there is one; run there; repeat.
                    let away_from = match npc.target {
                        Some(t) => target_pos(t, player_center, &blockers.list),
                        None => me.center + DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0) * -100.0,
                    };
                    let arrived = npc
                        .flee_dest
                        .map_or(true, |d| (d - me.center).truncate().length() < 40.0);
                    npc.flee_repick -= dt;
                    if arrived || npc.flee_repick <= 0.0 {
                        // Burning.PickDestination: water first - the nearest navigation point in a water zone
                        // within 2000, if this pawn can swim.
                        let water = npc.can_swim.then(|| {
                            lvl.nav
                                .points
                                .iter()
                                .map(|p| DVec3::from_array(p.pos))
                                .filter(|p| p.distance(me.center) < 2000.0 && crate::actors::water_at(lvl, *p).is_some())
                                .min_by(|a, b| a.distance(me.center).total_cmp(&b.distance(me.center)))
                        }).flatten();
                        let r = npc.rand();
                        let r2 = npc.rand();
                        let mut d = match water {
                            Some(w) => w,
                            None => pick_flee_dest(lvl, me.center, away_from, npc.params.radius, r, r2),
                        };
                        // Else run willy-nilly: about 300 units (x 0.8 - 1.2) off, away from the enemy.
                        if water.is_none() && d.distance(me.center) > 400.0 {
                            d = me.center + (d - me.center).normalize_or_zero() * 300.0 * (0.8 + 0.4 * r2 as f64);
                        }
                        npc.flee_dest = Some(d);
                        npc.route = None;
                        npc.flee_repick = 3.0;
                    }
                    let dest = npc.flee_dest.unwrap();
                    if npc.route.is_none() {
                        npc.route = Some(plan_route(lvl, me.center, dest, &npc.params, None));
                    }
                    let r = npc.route.as_mut().unwrap();
                    wish = follow(lvl, r, me.center, &npc.params, dt);
                    running = true;
                    if wish.length_squared() > 1e-6 {
                        npc.yaw =
                            turn_toward(npc.yaw, wish.y.atan2(wish.x), npc.yaw_rate * dt as f64);
                    }
                }
            }
            NpcState::Fleeing => {
                npc.weapon_drawn = false;
                let target_dead = match npc.target {
                    Some(Target::Pawn(b)) => *hp.0.get(&b).unwrap_or(&1.0) <= 0.0,
                    Some(Target::Player) => ph.dead,
                    None => true,
                };
                // Fleeing.Tick: from an enemy, until it's been out of sight for FearSustainTime; from something
                // merely frightening, until no longer fearful.
                let from_enemy = npc.target.is_some_and(|t| valid_enemy(npc, t));
                let done = target_dead
                    || if from_enemy || !npc.fear_flee {
                        now - npc.last_seen > npc.fear_sustain
                    } else {
                        !npc.fear.fearful()
                    };
                if done {
                    npc.fear_flee = false;
                    // FinishFleeing: back to orders.
                    npc.state = NpcState::Idle;
                    npc.target = None;
                    npc.route = None;
                    npc.flee_dest = None;
                    npc.patrol_dest = None;
                    npc.orders_failed = false;
                } else {
                    let enemy_pos = target_pos(
                        npc.target.unwrap_or(Target::Player),
                        player_center,
                        &blockers.list,
                    );
                    // bCower: crouch and cover up where it stands, facing the danger (Fleeing's Cower label).
                    if npc.cower {
                        npc.route = None;
                        let to = enemy_pos - me.center;
                        npc.yaw = turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64);
                    } else {
                    let seen = now - npc.last_seen < 0.6;
                    let arrived = npc
                        .flee_dest
                        .map_or(true, |d| (d - me.center).truncate().length() < 48.0);
                    npc.flee_repick -= dt;
                    // PickDestination when starting, when the hiding place turns out to be in view, or on arrival
                    // while still seen.
                    if npc.flee_dest.is_none() || (seen && (arrived || npc.flee_repick <= 0.0)) {
                        let r = npc.rand();
                        let r2 = npc.rand();
                        npc.flee_dest = Some(pick_flee_dest(
                            lvl,
                            me.center,
                            enemy_pos,
                            npc.params.radius,
                            r,
                            r2,
                        ));
                        npc.route = None;
                        npc.flee_repick = 2.0;
                    }
                    let dest = npc.flee_dest.unwrap();
                    if !arrived {
                        let replan = npc
                            .route
                            .as_ref()
                            .map_or(true, |r| r.age > 1.5 || r.goal.distance(dest) > 32.0);
                        if replan {
                            npc.route = Some(plan_route(lvl, me.center, dest, &npc.params, None));
                        }
                        let r = npc.route.as_mut().unwrap();
                        wish = follow(lvl, r, me.center, &npc.params, dt);
                        running = true;
                        if wish.length_squared() > 1e-6 {
                            npc.yaw = turn_toward(
                                npc.yaw,
                                wish.y.atan2(wish.x),
                                npc.yaw_rate * dt as f64,
                            );
                        }
                    } else {
                        // Hidden: wait, facing the way we came.
                        npc.route = None;
                        let to = enemy_pos - me.center;
                        npc.yaw = turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64);
                    }
                    }
                }
            }
            NpcState::Alerting => {
                // Alerting: weapon away, run to the alarm, push it, then fight (or flee).
                npc.weapon_drawn = false;
                running = true;
                let me_c = me.center;
                match npc.alert {
                    None => {
                        npc.state = if npc.raise_alarm == 0 {
                            NpcState::Attacking
                        } else {
                            NpcState::Fleeing
                        };
                    }
                    Some((i, push)) => {
                        let (spot, in_range, alarm_pos) = {
                            let u = &alarms.units[i];
                            (
                                u.push_spot(npc.params.radius),
                                u.in_range(me_c, npc.params.radius, npc.params.height),
                                u.pos,
                            )
                        };
                        if push > 0.0 || in_range {
                            // SoundAlarm: TurnToward, PlayPushing (PushButton), FinishAnim, TriggerAlarm.
                            npc.route = None;
                            let to = alarm_pos - me_c;
                            npc.yaw =
                                turn_toward(npc.yaw, to.y.atan2(to.x), npc.yaw_rate * dt as f64);
                            let left = if push > 0.0 {
                                push - dt
                            } else {
                                let ma = &lvl.mesh_actors[an.mesh_actor];
                                let clip = ma
                                    .anim
                                    .as_ref()
                                    .and_then(|a| Clip::named(&a.mesh, "PushButton", false));
                                let len = clip
                                    .as_ref()
                                    .map(|c| (c.frames as f32 - 1.0).max(1.0) / c.rate.max(1.0))
                                    .unwrap_or(1.0);
                                if let Some(c) = clip {
                                    an.clip = Some(c);
                                    npc.anim = "PushButton";
                                }
                                len.max(0.2)
                            };
                            if left <= 0.0 {
                                npc.alarm_timer = 120.0;
                                // TriggerAlarm: only if it hasn't been hacked; GMDX Sabotage shocks whoever
                                // pushes a bypassed one (200 'Stunned').
                                if !alarms.units[i].bypassed {
                                    alarms.trigger(i, npc.target);
                                } else if crate::perks::has("PerkSabotage") {
                                    new_hits.push(crate::damage::PawnHit {
                                        blocker: npc.blocker,
                                        damage: 200.0,
                                        damage_type: "Stunned",
                                        hit_loc: me_c,
                                        attacker: Some(npc.blocker),
                                        attacker_pos: me_c,
                                    });
                                }
                                alarms.units[i].associated = None;
                                npc.alert = None;
                                npc.anim = "";
                                if let Some(at) = npc.body_alarm.take() {
                                    // Pushed because of a body: now search there.
                                    npc.set_seek_location(at, SeekKind::Carcass, false);
                                    npc.enter_seeking();
                                } else if npc.raise_alarm == 0 {
                                    npc.state = NpcState::Attacking;
                                } else {
                                    npc.enter_fleeing();
                                }
                            } else {
                                npc.alert = Some((i, left));
                            }
                        } else {
                            let replan = npc
                                .route
                                .as_ref()
                                .map_or(true, |r| r.age > 2.0 || r.goal.distance(spot) > 32.0);
                            if replan {
                                npc.route = Some(plan_route(lvl, me_c, spot, &npc.params, None));
                            }
                            let r = npc.route.as_mut().unwrap();
                            wish = follow(lvl, r, me_c, &npc.params, dt);
                            if wish.length_squared() > 1e-6 {
                                npc.yaw = turn_toward(
                                    npc.yaw,
                                    wish.y.atan2(wish.x),
                                    npc.yaw_rate * dt as f64,
                                );
                            } else if r.age > 1.0 {
                                // Can't get there: give up on the alarm.
                                alarms.units[i].associated = None;
                                npc.alert = None;
                            }
                        }
                    }
                }
            }
            NpcState::Attacking => {
                // HandlingEnemy with RAISEALARM_BeforeAttacking: an alarm to push comes first.
                if std::mem::take(&mut npc.alert_pending) {
                    let me_b = npc.blocker;
                    let al = npc.alliances.clone();
                    if let Some(i) = alarms.find_target(me_b, me.center, |a| al.hostile_to(a)) {
                        alarms.units[i].associated = Some(me_b);
                        npc.alert = Some((i, 0.0));
                        npc.state = NpcState::Alerting;
                        npc.route = None;
                        println!("PAWN {} runs for the alarm", lvl.actors[npc.actor].name);
                    }
                }
                // Attacking.BeginState: SetupWeapon(true) - the weapon comes out even when no surprise pause ran.
                npc.weapon_drawn = true;
                let target_dead = match npc.target {
                    Some(Target::Pawn(b)) => *hp.0.get(&b).unwrap_or(&1.0) <= 0.0,
                    Some(Target::Player) => ph.dead,
                    None => true,
                };
                if target_dead {
                    // Nobody left to fight: back to orders (patrols pick up at the closest point again).
                    npc.state = NpcState::Idle;
                    npc.target = None;
                    npc.route = None;
                    npc.patrol_dest = None;
                    npc.orders_failed = false;
                } else if npc.should_flee()
                    || (npc.weapon.is_none() && !npc.arsenal.is_empty())
                    || npc.should_drop_weapon() && npc.weapon.is_some()
                {
                    // HandleEnemy: drop the gun if an arm is gone (ShouldDropWeapon), then run if badly hurt
                    // (bCriticalDamage) or out of ammo.
                    if npc.should_drop_weapon() {
                        if let Some(w) = npc.weapon.take() {
                            weapon_drops.0.push((
                                w.class.clone(),
                                me.center
                                    + DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0)
                                        * (me.radius + 8.0),
                            ));
                            npc.arsenal.retain(|x| x.class != w.class);
                            npc.dropped.push(w.class.clone());
                        }
                    }
                    if npc.should_flee() || npc.weapon.is_none() {
                        npc.enter_fleeing();
                        // RAISEALARM_BeforeFleeing: push an alarm on the way out if there's one to reach.
                        if npc.raise_alarm == 1 && npc.alarm_timer <= 0.0 {
                            let me_b = npc.blocker;
                            let al = npc.alliances.clone();
                            if let Some(i) =
                                alarms.find_target(me_b, me.center, |a| al.hostile_to(a))
                            {
                                alarms.units[i].associated = Some(me_b);
                                npc.alert = Some((i, 0.0));
                                npc.state = NpcState::Alerting;
                                npc.route = None;
                                println!(
                                    "PAWN {} runs for the alarm before fleeing",
                                    lvl.actors[npc.actor].name
                                );
                            }
                        }
                    }
                } else if now - npc.last_seen > npc.enemy_timeout {
                    // HasEnemyTimedOut: search where they are now (SEEKTYPE_Guess, post-combat).
                    let at = target_pos(
                        npc.target.unwrap_or(Target::Player),
                        player_center,
                        &blockers.list,
                    );
                    npc.set_seek_location(at, SeekKind::Guess, true);
                    npc.enter_seeking();
                } else {
                    let target_pos = match npc.target {
                        Some(Target::Player) => player_center,
                        Some(Target::Pawn(b)) => blockers.list[b].center,
                        None => npc.last_pos,
                    };
                    let seen_recently = now - npc.last_seen < 0.6;
                    let chase_to = if seen_recently {
                        target_pos
                    } else {
                        npc.last_pos
                    };
                    let to = chase_to - me.center;
                    let dist = to.length();
                    // Every half second (and when the gun runs dry) pick the best weapon for this range.
                    npc.switch_check -= dt;
                    if npc.switch_check <= 0.0 && npc.reload_timer <= 0.0 {
                        npc.switch_check = 0.5;
                        let enemy = match npc.target {
                            Some(Target::Player) => Some((dist, player.params.radius, false, true)),
                            Some(Target::Pawn(b)) => {
                                let c = blockers.list[b];
                                Some((
                                    dist,
                                    c.radius,
                                    find(&lvl.actors[c.actor].props, "EMPHitPoints").is_some(),
                                    false,
                                ))
                            }
                            None => None,
                        };
                        npc.switch_to_best_weapon(enemy);
                    }
                    let melee = npc.weapon.as_ref().map_or(false, |w| w.hand_to_hand);
                    // Melee weapons close right in; guns hold off at a fraction of their range.
                    let accurate = match npc.weapon.as_ref() {
                        Some(w) if w.hand_to_hand => {
                            (w.max_range as f64 * 0.8).max(me.radius + 20.0)
                        }
                        Some(w) => w.max_range as f64 * 0.4,
                        None => 600.0,
                    };
                    // ShouldCrouch: a ranged weapon, out of the water, the enemy more than 300 away.
                    let should_crouch = npc.can_crouch
                        && !melee
                        && dist > 300.0
                        && crate::actors::water_at(lvl, me.center).is_none();
                    if npc.crouching {
                        npc.crouch_timer = (npc.crouch_timer - dt).max(0.0);
                        if npc.crouch_timer <= 0.0 && !should_crouch {
                            npc.crouching = false;
                        }
                    }
                    if !seen_recently || dist > accurate {
                        npc.crouch_rolled = false;
                    } else if !npc.crouch_rolled {
                        // Attacking's Fire label: ShouldCrouch() && FRand() < CrouchRate -> StartCrouch.
                        npc.crouch_rolled = true;
                        if should_crouch && npc.rand() < npc.crouch_rate {
                            npc.crouching = true;
                            npc.crouch_timer = 1.0 + npc.rand() * 0.5;
                        }
                    }
                    if npc.crouching && npc.crouch_timer > 0.0 {
                        // PickDestination: DEST_SameLocation while the crouch timer runs.
                        npc.route = None;
                    } else if npc.crouching && (!seen_recently || dist > accurate) {
                        npc.crouching = false;
                    }
                    if !npc.crouching && (!seen_recently || dist > accurate) {
                        // Close in, over the path network if there's no straight way.
                        let replan = npc
                            .route
                            .as_ref()
                            .map_or(true, |r| r.age > 1.0 || r.goal.distance(chase_to) > 96.0);
                        if replan {
                            npc.route =
                                Some(plan_route(lvl, me.center, chase_to, &npc.params, None));
                        }
                        let r = npc.route.as_mut().unwrap();
                        wish = follow(lvl, r, me.center, &npc.params, dt);
                        running = true;
                    } else {
                        npc.route = None;
                    }
                    // Attacking's labels (atk_phase): RunToRange opens with TurnToward(enemy) - standing, no shots -
                    // then runs; stopping goes through Fire (TweenToShoot 0.15 s + TurnToward) into ContinueFire.
                    let moving = wish.length_squared() > 1e-6;
                    npc.atk_phase = match (npc.atk_phase, moving) {
                        (AtkPhase::Moving | AtkPhase::TurnFirst, true) => npc.atk_phase,
                        (_, true) => AtkPhase::TurnFirst,
                        (AtkPhase::Moving | AtkPhase::TurnFirst, false) => AtkPhase::Settle(0.15),
                        (AtkPhase::Settle(t), false) => AtkPhase::Settle(t - dt),
                        (AtkPhase::Firing, false) => AtkPhase::Firing,
                    };
                    if npc.atk_phase == AtkPhase::TurnFirst {
                        wish = DVec3::ZERO;
                        running = false;
                    }
                    // StrafeFacing (bCanStrafe and a shot to take) runs facing the enemy; MoveToward faces the way it
                    // goes; standing, turning and firing face the enemy.
                    let strafing = npc.can_strafe && seen_recently;
                    let face = if !moving || npc.atk_phase == AtkPhase::TurnFirst || strafing {
                        to
                    } else {
                        wish
                    };
                    let want = face.y.atan2(face.x);
                    npc.yaw = turn_toward(npc.yaw, want, npc.yaw_rate * dt as f64);
                    let to_t = target_pos - me.center;
                    let want_t = to_t.y.atan2(to_t.x);
                    // TurnToward is done within 2000 rotation units (about 11 degrees).
                    let facing_ok =
                        (want_t - npc.yaw).sin().abs() < 0.2 && (want_t - npc.yaw).cos() > 0.0;
                    let off_deg = (want_t - npc.yaw).sin().atan2((want_t - npc.yaw).cos()).abs().to_degrees() as f32;
                    match npc.atk_phase {
                        AtkPhase::TurnFirst if facing_ok => npc.atk_phase = AtkPhase::Moving,
                        AtkPhase::Settle(t) if t <= 0.0 && facing_ok => npc.atk_phase = AtkPhase::Firing,
                        _ => {}
                    }
                    // Attacking.Tick / ContinueFire: who may shoot now.
                    let fire_ok = match npc.atk_phase {
                        AtkPhase::TurnFirst | AtkPhase::Settle(_) => false,
                        // bFacingTarget while strafing; else !bMustFaceTarget within FireAngle (Abs(yaw)*360/32768).
                        AtkPhase::Moving => {
                            if strafing {
                                facing_ok
                            } else {
                                !npc.must_face && off_deg * 2.0 <= npc.fire_angle
                            }
                        }
                        AtkPhase::Firing => facing_ok,
                    };
                    // Karkian.Bump: charging into its enemy faster than 100 with the ram (WeaponKarkianBump) out and
                    // FireTimer clear hits twice for speed/5 'Shot', then FireTimer = AIFireDelay (8 s of biting).
                    if lvl.actors[npc.actor].class == "Karkian"
                        && npc.target == Some(Target::Player)
                        && !ph.dead
                        && npc.fire_timer <= 0.0
                    {
                        if let Some(delay) = npc
                            .weapon
                            .as_ref()
                            .filter(|w| w.class == "WeaponKarkianBump")
                            .map(|w| w.ai_fire_delay)
                        {
                            let v = npc.body.as_ref().map_or(DVec3::ZERO, |b| b.velocity);
                            let flat = (player_center - me.center).truncate().length();
                            let touching = flat <= me.radius + player.params.radius + 6.0
                                && (player_center.z - me.center.z).abs()
                                    <= me.height + player.params.height;
                            if touching && v.length() > 100.0 {
                                let dmg = (v.length() / 5.0) as f32;
                                for o in [DVec3::new(1.0, 1.0, -1.0), DVec3::new(-1.0, -1.0, -1.0)]
                                {
                                    ph.damage(
                                        dmg,
                                        "Shot",
                                        player_offset(player_center, player_yaw, player_center + o),
                                    );
                                }
                                crate::shake::shake_view(0.15 + 0.002 * dmg * 2.0, dmg * 30.0 * 2.0, 0.3 * dmg * 2.0);
                                npc.fire_timer = delay;
                                npc.switch_check = 0.0;
                            }
                        }
                    }
                    firing = npc.weapon.is_some();
                    npc.shot_timer -= dt;
                    // ContinueFire: the next shot waits for the Shoot animation to finish (FinishAnim) and for the
                    // weapon to be ready again (ShotTime), so fire rate is the slower of the two.
                    let shoot_playing = npc.anim == "ShootOnce"
                        && an.clip.as_ref().map(|c| !c.done()).unwrap_or(false);
                    if npc.reload_timer > 0.0 {
                        npc.reload_timer -= dt;
                        if npc.reload_timer <= 0.0 {
                            if let Some(snd) =
                                npc.weapon.as_ref().and_then(|w| w.snd.reload_end.clone())
                            {
                                crate::sound::play(&snd, me.center, 1.0, 1024.0, 1.0);
                            }
                            let left = npc.weapon_ammo().unwrap_or(i32::MAX);
                            npc.loaded = npc
                                .weapon
                                .as_ref()
                                .map(|w| (w.reload_count.max(1) as i32).min(left).max(0) as u32)
                                .unwrap_or(1);
                        }
                    } else if let Some(w) = npc.weapon.clone() {
                        let in_reach = !melee || dist <= w.max_range as f64 + me.radius;
                        let has_ammo = npc.weapon_ammo().map_or(true, |n| n > 0);
                        // Melee: the swing starts with the Attack animation and the blow lands at its
                        // HandToHandAttack notify (OwnerHandToHandAttack), still needing the target in reach.
                        let mut strike_now = false;
                        if let Some(t) = npc.melee_wait.as_mut() {
                            *t -= dt;
                            if *t <= 0.0 {
                                npc.melee_wait = None;
                                strike_now = in_reach;
                            }
                        }
                        if strike_now
                            || (seen_recently
                                && fire_ok
                                && in_reach
                                && has_ammo
                                && npc.shot_timer <= 0.0
                                && !shoot_playing
                                && npc.melee_wait.is_none())
                        {
                            'fire: {
                                if !strike_now {
                                    npc.shot_timer = w.shot_time.max(0.1);
                                    npc.just_fired = true;
                                    if std::env::var("DXVIEW_PAWNLOG").is_ok_and(|n| lvl.actors[npc.actor].name.eq_ignore_ascii_case(&n)) {
                                        println!("PAWNSHOT t={now:.2} {} phase={:?} moving={moving}", lvl.actors[npc.actor].name, npc.atk_phase);
                                    }
                                    if w.ai_fire_delay > 0.0 {
                                        // FireIfClearShot: FireTimer = AIFireDelay; the next SwitchToBestWeapon moves off it.
                                        npc.fire_timer = w.ai_fire_delay;
                                    }
                                    let fire_snd = if w.silenced && !w.hand_to_hand {
                                        Some("DeusExSounds.Weapons.StealthPistolFire".to_string())
                                    } else {
                                        w.snd.fire.clone()
                                    };
                                    if let Some(snd) = fire_snd {
                                        crate::sound::play(
                                            &snd,
                                            me.center,
                                            w.snd.volume,
                                            2048.0,
                                            1.0,
                                        );
                                    }
                                    if w.hand_to_hand {
                                        let seq = lvl.mesh_actors[an.mesh_actor]
                                            .anim
                                            .as_ref()
                                            .and_then(|a| {
                                                a.mesh
                                                    .seqs
                                                    .iter()
                                                    .find(|q| q.name.eq_ignore_ascii_case("Attack"))
                                                    .cloned()
                                            });
                                        let delay = seq.map_or(0.25, |q| {
                                            let at = q
                                                .notifies
                                                .iter()
                                                .find(|n| {
                                                    n.1.eq_ignore_ascii_case("HandToHandAttack")
                                                })
                                                .map_or(0.5, |n| n.0);
                                            q.num_frames.max(1) as f32 / q.rate.max(0.1) * at
                                        });
                                        npc.melee_wait = Some(delay);
                                        break 'fire;
                                    }
                                }
                                // Gunfire is heard by enemies (WeaponFire, LoudNoise) and friends (Distress).
                                if let Some(mut n) = crate::stealth::weapon_noise(
                                    &w,
                                    me.center,
                                    Target::Pawn(npc.blocker),
                                ) {
                                    new_noises.push(n.clone());
                                    n.kind = NoiseKind::Loud;
                                    new_noises.push(n.clone());
                                    n.kind = NoiseKind::Distress;
                                    new_noises.push(n);
                                }
                                // NPC ammo (user decision 2026-10-12): pawns never run dry and only the mini-crossbow
                                // (and any ReloadCount 1 weapon) reloads, every ReloadCount darts. DXVIEW_NPC_FINITE_AMMO=1 restores
                                // the script's behaviour (UseAmmo per shot, ReloadAmmo every ClipCount shots).
                                let finite = npc_finite_ammo();
                                let reloads = finite || w.reload_count == 1 || w.class == "WeaponMiniCrossbow";
                                if w.reload_count > 0 && reloads {
                                    if finite {
                                        npc.use_ammo();
                                    }
                                    npc.loaded = npc.loaded.saturating_sub(1);
                                    if npc.weapon_ammo() == Some(0) {
                                        // Out of ammo: switch to something else (a knife) straight away.
                                        npc.switch_check = 0.0;
                                    } else if npc.loaded == 0 {
                                        // Reload.GetReloadTime for pawns: ReloadTime * (BaseAccuracy * 2 + 1).
                                        npc.reload_timer =
                                            w.reload_time * (npc.base_accuracy * 2.0 + 1.0);
                                        if let Some(snd) = &w.snd.reload_begin {
                                            crate::sound::play(snd, me.center, 1.0, 1024.0, 1.0);
                                        }
                                    }
                                }
                                // Aim at the target's chest with weapon + pawn inaccuracy.
                                let aim_at = target_pos + DVec3::new(0.0, 0.0, 10.0);
                                let fwd = (aim_at - my_eye).normalize_or_zero();
                                let right = fwd.cross(DVec3::Z).normalize_or_zero();
                                let up = right.cross(fwd).normalize_or_zero();
                                let acc = (w.base_accuracy + npc.base_accuracy) as f64;
                                let mut continue_fire = true;
                                if let Some(class) = w.projectile_for(npc.ammo.as_deref()) {
                                    for _ in 0..w.shots_per_fire() {
                                        let dir = crate::projectiles::projectile_dir(
                                            fwd,
                                            acc,
                                            &mut npc.seed,
                                        );
                                        launches.0.push(crate::projectiles::Launch {
                                            class: class.clone(),
                                            pos: my_eye + fwd * (me.radius + 4.0),
                                            dir,
                                            from: Some(npc.blocker),
                                            mult: 1.0,
                                            life: None,
                                            scale: None,
                                            still: false,
                                            track: None,
                                            track_at: None,
                                            speed: None,
                                            ownerless: false,
                                        });
                                    }
                                    continue_fire = false;
                                }
                                let mut ricocheted = false;
                                for _ in 0..if continue_fire { w.shots_per_fire() } else { 0 } {
                                    let range = w.max_range as f64;
                                    let dir = spread_dir(fwd, right, up, acc, range, &mut npc.seed);
                                    let end = my_eye + dir * range;
                                    let wh = world.trace(my_eye, end, DVec3::splat(0.5));
                                    let mut ph_hit =
                                        trace_cylinder(&player_cyl, my_eye, end, DVec3::splat(0.5));
                                    // Enhancement (Carried Objects As Cover, carry.rs): what you hold in front of
                                    // you takes the bullets meant for you (the original turns its collision off).
                                    if let Some(hc) = carrying.held.as_ref().filter(|h| crate::carry::cover_on() && h.mass >= 15.0 && !w.hand_to_hand) {
                                        let box_cyl = Cylinder {
                                            center: blockers.list[hc.blocker].center,
                                            radius: hc.radius,
                                            height: hc.height,
                                            actor: usize::MAX,
                                        };
                                        if let Some(c) = trace_cylinder(&box_cyl, my_eye, end, DVec3::splat(0.5)) {
                                            let nearer_than_player = ph_hit.is_none_or(|p| c.fraction <= p.fraction);
                                            if c.fraction < wh.fraction && nearer_than_player && !ph.dead {
                                                let at = my_eye + (end - my_eye) * c.fraction;
                                                crate::fx::impact(at, -dir, dir, Some("wood"));
                                                crate::sound::play(
                                                    &format!("DeusExSounds.Generic.Ricochet{}", 1 + (crate::sound::frand() * 4.0) as u32 % 4),
                                                    at,
                                                    0.8,
                                                    1024.0,
                                                    1.0,
                                                );
                                                deco_hits.0.push(crate::destruct::DecoHit {
                                                    blocker: hc.blocker,
                                                    damage: w.hit_damage,
                                                    damage_type: w.damage_type(npc.ammo.as_deref()),
                                                });
                                                ph_hit = None;
                                            }
                                        }
                                    }
                                    // TraceFire's tracer (10mm / 30.06 / 7.62mm, one slug, half the time).
                                    if !w.hand_to_hand
                                        && w.shots_per_fire() == 1
                                        && crate::impacts::tracer_ammo(w.ammo_class.as_deref())
                                        && crate::sound::frand() < 0.5
                                    {
                                        let f = match ph_hit {
                                            Some(p) if p.fraction < wh.fraction && !ph.dead => p.fraction,
                                            _ => wh.fraction,
                                        };
                                        crate::impacts::tracer(my_eye, end, my_eye + (end - my_eye) * f);
                                    }
                                    match ph_hit {
                                        Some(p) if p.fraction < wh.fraction && !ph.dead => {
                                            let dmg = w.hit_damage;
                                            if walk_test.is_some() {
                                                println!(
                                                    "AI {} hit player for {dmg:.1}",
                                                    lvl.actors[npc.actor].name
                                                );
                                            }
                                            let at = my_eye + (end - my_eye) * p.fraction;
                                            if w.hand_to_hand {
                                                if let Some(snd) = &w.snd.misc[0] {
                                                    crate::sound::play(
                                                        snd, me.center, 1.0, 1024.0, 1.0,
                                                    );
                                                }
                                            }
                                            let off = player_offset(player_center, player_yaw, at);
                                            ph.damage(dmg, w.damage_type(npc.ammo.as_deref()), off);
                                        }
                                        _ => {
                                            if wh.fraction < 1.0 {
                                                let at = my_eye + dir * range * wh.fraction;
                                                let props = wh.actor.map(|b| {
                                                    &lvl.actors[blockers.list[b].actor].props
                                                });
                                                let pawn = props.map_or(false, |p| {
                                                    find(p, "GroundSpeed").is_some()
                                                });
                                                let robot = props.map_or(false, |p| {
                                                    find(p, "EMPHitPoints").is_some()
                                                });
                                                if w.hand_to_hand {
                                                    let k = if pawn {
                                                        0
                                                    } else if props.is_some() {
                                                        2
                                                    } else {
                                                        1
                                                    };
                                                    if let Some(snd) = &w.snd.misc[k] {
                                                        crate::sound::play(
                                                            snd, me.center, 1.0, 1024.0, 1.0,
                                                        );
                                                    }
                                                } else if !ricocheted
                                                    && w.penetrating
                                                    && (!pawn || robot)
                                                    && matches!(
                                                        w.damage_type(npc.ammo.as_deref()),
                                                        "Shot" | "Sabot"
                                                    )
                                                {
                                                    ricocheted = true;
                                                    let armored = robot
                                                        || props.map_or(false, |p| {
                                                            crate::sound::fprop(
                                                                p,
                                                                "minDamageThreshold",
                                                                0.0,
                                                            ) > 10.0
                                                        });
                                                    let snd = if armored {
                                                        "DeusExSounds.Generic.ArmorRicochet"
                                                            .to_string()
                                                    } else {
                                                        format!(
                                                            "DeusExSounds.Generic.Ricochet{}",
                                                            1 + (crate::sound::frand() * 4.0)
                                                                as u32
                                                                % 4
                                                        )
                                                    };
                                                    crate::sound::play(
                                                        &snd,
                                                        at,
                                                        1.0,
                                                        1024.0,
                                                        1.1 - 0.2 * crate::sound::frand(),
                                                    );
                                                }
                                            }
                                            if wh.fraction < 1.0 && wh.actor.is_none() {
                                                crate::movers::hit_line(
                                                    my_eye,
                                                    my_eye + dir * range * wh.fraction,
                                                    w.hit_damage,
                                                    w.damage_type(npc.ammo.as_deref()),
                                                );
                                                crate::impacts::wall_hit(
                                                    my_eye + dir * range * wh.fraction,
                                                    wh.normal,
                                                    dir,
                                                    w.hit_damage,
                                                    crate::impacts::Source::Weapon {
                                                        penetrating: w.penetrating,
                                                        hand_to_hand: w.hand_to_hand,
                                                    },
                                                );
                                            }
                                            if let Some(b) = wh.actor {
                                                let other = blockers.list[b];
                                                let oa = &lvl.actors[other.actor];
                                                if find(&oa.props, "GroundSpeed").is_none() {
                                                    deco_hits.0.push(crate::destruct::DecoHit {
                                                        blocker: b,
                                                        damage: w.hit_damage,
                                                        damage_type: w
                                                            .damage_type(npc.ammo.as_deref()),
                                                    });
                                                } else {
                                                    if w.penetrating
                                                        && find(&oa.props, "EMPHitPoints").is_none()
                                                    {
                                                        blood.0.push((
                                                            my_eye + dir * range * wh.fraction
                                                                - dir,
                                                            dir,
                                                        ));
                                                    }
                                                    new_hits.push(crate::damage::PawnHit {
                                                        blocker: b,
                                                        damage: w.hit_damage,
                                                        damage_type: w
                                                            .damage_type(npc.ammo.as_deref()),
                                                        hit_loc: my_eye + dir * range * wh.fraction,
                                                        attacker: Some(npc.blocker),
                                                        attacker_pos: me.center,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // A medical/repair bot serving the player stands still with its arm raised.
        npc.service_cooldown = (npc.service_cooldown - dt).max(0.0);
        if npc.service_pause > 0.0 && npc.state != NpcState::Dead {
            npc.service_pause -= dt;
            wish = DVec3::ZERO;
            running = false;
            firing = false;
            let ma = &lvl.mesh_actors[an.mesh_actor];
            let mesh = ma.anim.as_ref().map(|a| a.mesh.clone());
            if !npc.service_shown {
                npc.service_shown = true;
                let raise = if lvl.actors[npc.actor].class == "RepairBot" {
                    "DeusExSounds.Robot.RepairBotRaiseArm"
                } else {
                    "DeusExSounds.Robot.MedicalBotRaiseArm"
                };
                crate::sound::play(raise, me.center, 1.0, 512.0, 1.0);
                if let Some(c) = mesh.as_ref().and_then(|m| Clip::named(m, "Start", false)) {
                    an.clip = Some(c);
                    npc.anim = "Start";
                }
            }
            if npc.service_pause <= 0.0 {
                npc.service_shown = false;
                let lower = if lvl.actors[npc.actor].class == "RepairBot" {
                    "DeusExSounds.Robot.RepairBotLowerArm"
                } else {
                    "DeusExSounds.Robot.MedicalBotLowerArm"
                };
                crate::sound::play(lower, me.center, 1.0, 512.0, 1.0);
                if let Some(c) = mesh.as_ref().and_then(|m| Clip::named(m, "Stop", false)) {
                    an.clip = Some(c);
                    npc.anim = "Stop";
                }
            }
        }
        // AvoidingProjectiles: run clear of a live grenade first, whatever else is going on.
        if npc.avoiding > 0.0 {
            npc.avoiding -= dt;
            if !matches!(
                npc.state,
                NpcState::Dead
                    | NpcState::Stunned(_)
                    | NpcState::RubbingEyes(_)
                    | NpcState::TakingHit(_)
            ) {
                if let Some(dest) = npc.flee_dest {
                    if (dest - me.center).truncate().length() > 24.0 {
                        let replan = npc
                            .route
                            .as_ref()
                            .map_or(true, |r| r.goal.distance(dest) > 32.0);
                        if replan {
                            npc.route = Some(plan_route(lvl, me.center, dest, &npc.params, None));
                        }
                        let r = npc.route.as_mut().unwrap();
                        wish = follow(lvl, r, me.center, &npc.params, dt);
                        running = true;
                        firing = false;
                        if wish.length_squared() > 1e-6 {
                            npc.yaw = turn_toward(
                                npc.yaw,
                                wish.y.atan2(wish.x),
                                npc.yaw_rate * dt as f64,
                            );
                        }
                    }
                }
            }
            if npc.avoiding <= 0.0 && npc.state != NpcState::Fleeing {
                npc.flee_dest = None;
                npc.route = None;
            }
        }
        if npc.state != NpcState::Attacking {
            npc.crouching = false;
            npc.crouch_rolled = false;
        }
        if npc.state == NpcState::Idle {
            // SetupWeapon(false): holstered outside combat unless bKeepWeaponDrawn.
            npc.weapon_drawn = npc.keep_drawn;
        }
        if npc.seek.is_some()
            && npc.state != NpcState::Seeking
            && !matches!(
                npc.state,
                NpcState::TakingHit(_) | NpcState::Stunned(_) | NpcState::RubbingEyes(_)
            )
        {
            npc.seek = None;
            npc.seek_level = 0;
        }

        // DXVIEW_PAWNLOG=<actor name>: where that pawn is and what it's doing, twice a second.
        if let Ok(want) = std::env::var("DXVIEW_PAWNLOG") {
            if lvl.actors[npc.actor].name.eq_ignore_ascii_case(&want) && (now * 2.0).floor() != ((now - dt) * 2.0).floor() {
                let c = blockers.list[npc.blocker].center;
                println!(
                    "PAWNLOG t={:.1} {} at ({:.0},{:.0},{:.0}) yaw={:.0} state={:?} phase={:?} anim={} wish={:.2?} route={} target={:?} fired={}",
                    now, lvl.actors[npc.actor].name, c.x, c.y, c.z, npc.yaw.to_degrees(), npc.state, npc.atk_phase, npc.anim, wish.truncate(),
                    npc.route.is_some(), npc.target, npc.just_fired
                );
            }
        }
        if ai_log && std::mem::discriminant(&npc.state) != state_before {
            println!(
                "AI t={:.1} {} -> {:?} readiness={:.2} reaction={:.2} seek={:?} target={:?}",
                now,
                lvl.actors[npc.actor].name,
                npc.state,
                npc.readiness,
                npc.reaction,
                npc.seek.as_ref().map(|s| (s.kind, s.pos.round())),
                npc.target
            );
        }
        // StartCrouch / EndCrouch: SetBasedPawnSize keeps the feet where they are, so the centre moves by the change
        // in height; standing back up needs headroom.
        if npc.state != NpcState::Dead && !npc.sitting {
            let want_h = if npc.crouching {
                npc.stand_height * 0.65
            } else {
                npc.stand_height
            };
            let delta = want_h - npc.params.height;
            if delta.abs() > 0.01 {
                let at = npc.body.as_ref().map_or(deco.body.location, |b| b.location);
                let room = delta < 0.0
                    || lvl
                        .collision
                        .trace(
                            at,
                            at + DVec3::new(0.0, 0.0, 2.0 * delta),
                            DVec3::new(npc.params.radius, npc.params.radius, npc.params.height),
                        )
                        .fraction
                        >= 1.0;
                if room {
                    npc.params.height = want_h;
                    if let Some(b) = npc.body.as_mut() {
                        b.location.z += delta;
                    }
                    deco.body.location.z += delta;
                    deco.body.height = want_h;
                    deco.pre.z -= delta;
                    blockers.list[npc.blocker].height = want_h;
                }
            }
        }
        // Move: walking pace for orders, full speed in combat.
        let mut params = npc.params.clone();
        if !running {
            params.ground_speed *= npc.walk_frac;
        }
        // State OpeningDoor: back off out of the door's swing (StrafeTo(destLoc)), frob it (before backing up when
        // it doesn't encroach, after when it does), then wait for it to finish opening (StopWaiting) or 5 s.
        if let Some(dw) = npc.door.as_mut() {
            dw.t += dt;
            let here = npc.body.as_ref().map_or(deco.body.location, |b| b.location);
            let to = DVec3::new(dw.back_to.x - here.x, dw.back_to.y - here.y, 0.0);
            let backing = !dw.frobbed || dw.encroaches && dw.t < 2.0;
            let mut finished = false;
            if backing && to.length() > 6.0 && dw.t < 2.0 {
                wish = to.normalize_or_zero();
                running = true;
            } else {
                wish = DVec3::ZERO;
                if !dw.frobbed {
                    dw.frobbed = true;
                    dw.t = 0.0;
                    if !frob_door(lvl, &mv, dw.mover, npc.blocker) {
                        finished = true;
                    }
                } else {
                    let rt = &mv.rt[dw.mover];
                    // FinishNotify -> StopWaiting once the door is at rest open; Sleep(5) otherwise.
                    if (dw.t > 0.3 && !rt.interpolating && rt.key_num > 0) || dw.t > 5.0 || rt.destroyed {
                        finished = true;
                    }
                }
            }
            if finished {
                npc.door = None;
                npc.stall = 0.0;
                npc.route = None;
            }
        }
        let body = npc.body.as_mut().unwrap();
        let before = body.location;
        if !npc.sitting {
            let steps = ((dt as f64) / (1.0 / 60.0)).ceil().max(1.0) as usize;
            // Pawns can't walk through the player either.
            let mut solid = blockers.list.clone();
            if !ph.dead {
                solid.push(player_cyl);
            }
            // BlockMonsters stop pawns only.
            solid.extend(lvl.monster_blocks.iter().map(|c| Cylinder { actor: usize::MAX, ..c.clone() }));
            // ...nor through what the player is carrying.
            if let Some(h) = carrying.held.as_ref() {
                solid.push(Cylinder {
                    center: blockers.list[h.blocker].center,
                    radius: h.radius,
                    height: h.height,
                    actor: usize::MAX,
                });
            }
            let world = World {
                level: lvl,
                blockers: &solid,
                ignore: Some(npc.blocker),
                pushable: None,
                movers: Some(&mv.rt),
            };
            for _ in 0..steps {
                body.tick(&world, &params, wish, false, dt as f64 / steps as f64);
            }
        } else {
            if let Some((seat_actor, bl, off)) = npc.seat.as_mut() {
                if bl.is_none() {
                    *bl = blockers.list.iter().position(|c| c.actor == *seat_actor);
                }
                if let Some(c) = bl.and_then(|b| blockers.list.get(b)) {
                    deco.body.location = c.center + *off;
                    if let Some(me) = blockers.list.get_mut(npc.blocker) {
                        me.center = deco.body.location;
                    }
                }
            }
            body.location = deco.body.location;
            body.velocity = DVec3::ZERO;
        }
        let moved = body.location;
        let mut vel = body.velocity;
        // HitWall -> CheckOpenDoor: walking into a mover. Glass that isn't a door gets broken (TakeDamage 200);
        // a door (more than one key and bIsDoor) is opened - by its button if a decoration triggers it, else by
        // Frob, which lets any pawn but the player through any door, locked or not.
        if npc.door.is_none()
            && npc.can_open_doors
            && !npc.sitting
            && wish.truncate().length_squared() > 0.01
            && matches!(
                npc.state,
                NpcState::Idle | NpcState::Attacking | NpcState::Seeking | NpcState::Fleeing | NpcState::Alerting | NpcState::Burning
            )
        {
            let dir = DVec3::new(wish.x, wish.y, 0.0).normalize_or_zero();
            let ext = DVec3::new(npc.params.radius - 1.0, npc.params.radius - 1.0, npc.params.height - 1.0);
            let mut best: Option<(usize, f64, DVec3)> = None;
            for (i, (info, rt)) in lvl.movers.iter().zip(mv.rt.iter()).enumerate() {
                if rt.destroyed || rt.location.distance(moved) > 1500.0 {
                    continue;
                }
                if let Some(h) = crate::movers::trace_mover(info, rt, moved, moved + dir * 6.0, ext) {
                    if best.map_or(true, |b| h.fraction < b.1) {
                        best = Some((i, h.fraction, h.normal));
                    }
                }
            }
            if let Some((i, _, normal)) = best {
                let props = &lvl.actors[lvl.movers[i].actor].props;
                let rt = &mv.rt[i];
                let door = is_door(lvl, i);
                let breakable = rt.breakable.unwrap_or(matches!(find(props, "bBreakable"), Some(Value::Bool(true))));
                let ignore_encroach = matches!(find(props, "MoverEncroachType"), Some(Value::Byte(3)));
                if !door && breakable {
                    crate::movers::damage_mover(i, 200.0, "Shot");
                } else if door && !(rt.interpolating && ignore_encroach) {
                    let back_to = backup_point(lvl, &mv, i, moved, normal, npc.params.radius, npc.params.height);
                    let mut dw = DoorWait { mover: i, back_to, t: 0.0, frobbed: false, encroaches: !ignore_encroach };
                    if !dw.encroaches {
                        dw.frobbed = true;
                        if frob_door(lvl, &mv, i, npc.blocker) {
                            npc.door = Some(dw);
                        }
                    } else {
                        npc.door = Some(dw);
                    }
                    if npc.door.is_some() {
                        npc.stall = 0.0;
                        if std::env::var("DXVIEW_DOORLOG").is_ok() {
                            println!("DOOR {} opens {} (locked {})", lvl.actors[npc.actor].name, lvl.movers[i].name, rt.locked);
                        }
                    }
                }
            }
        }
        // Stall watchdog: wanting to move but getting nowhere (a pawn, a door, a ledge the route didn't know
        // about) for 1.5 s drops the route and gives up the current leg, so nobody runs on the spot. Speed for
        // the animation is what was actually covered, not the velocity pushing into the obstacle.
        let covered = (moved - before).truncate().length() / (dt as f64).max(1e-3);
        if wish.length_squared() > 0.01 && covered < 15.0 && !npc.sitting {
            npc.stall += dt;
        } else {
            npc.stall = 0.0;
        }
        if covered < 15.0 {
            vel = DVec3::new(0.0, 0.0, vel.z);
        }
        if npc.stall > 1.5 {
            npc.stall = 0.0;
            npc.stalls += 1;
            npc.route = None;
            match npc.state {
                NpcState::Idle => {
                    npc.patrol_dest = None;
                    npc.wander_goal = None;
                    npc.pause = 0.5 + npc.rand();
                    if npc.stalls >= 3 && matches!(npc.orders, Orders::RunningTo | Orders::GoingTo) {
                        // OrdersFailed: stand where we got to.
                        npc.orders = Orders::Standing;
                        npc.home = moved;
                        npc.home_yaw = npc.yaw;
                    }
                }
                NpcState::Seeking => {
                    if npc.stalls >= 2 {
                        npc.finish_seek();
                    }
                }
                _ => {}
            }
        } else if covered > 30.0 {
            npc.stalls = 0;
        }
        let speed = vel.truncate().length();
        // Outside combat, face the way we're walking.
        if matches!(npc.state, NpcState::Idle | NpcState::Seeking) && speed > 20.0 {
            npc.yaw = turn_toward(npc.yaw, vel.y.atan2(vel.x), npc.yaw_rate * dt as f64);
        }
        deco.body.location = moved;
        blockers.list[npc.blocker].center = moved;
        let yaw_units = (npc.yaw / std::f64::consts::TAU * 65536.0) as i32;
        deco.rotation = [0, yaw_units, 0];
        let draw = moved + deco.pre;
        tf.translation = Vec3::from(ue_to_bevy([draw.x as f32, draw.y as f32, draw.z as f32]));
        let m = rotator_to_bevy_mat3([0, yaw_units, 0]);
        tf.rotation = Quat::from_mat3(&Mat3::from_cols(
            Vec3::from(m[0]),
            Vec3::from(m[1]),
            Vec3::from(m[2]),
        ));

        if walk_test.is_some()
            && (*test_clock / 2.0).floor() != ((*test_clock - dt) / 2.0).floor()
            && npc.orders != Orders::Standing
        {
            println!(
                "NAV t={:.0} {} {:?} {:?} at ({:.0},{:.0},{:.0}) dest={:?} route={} anim={} drawn={}",
                *test_clock,
                lvl.actors[npc.actor].name,
                npc.orders,
                npc.state,
                moved.x,
                moved.y,
                moved.z,
                npc.patrol_dest.map(|d| lvl.nav.points[d].tag.clone()),
                npc.route.as_ref().map(|r| r.nodes.len() as i32).unwrap_or(-1),
                npc.anim,
                npc.weapon_drawn
            );
        }
        // Animation (DXVIEW_POSE=<sequence> forces one pose on every pawn, for visual checks).
        if let Some(pose) = &pose {
            let name: &'static str = Box::leak(pose.clone().into_boxed_str());
            if npc.anim != pose.as_str() {
                set_anim(&mut an, &level, npc, name, "", 1.0);
            }
            continue;
        }
        if matches!(npc.state, NpcState::TakingHit(_)) {
            continue;
        }
        if matches!(npc.state, NpcState::Stunned(_)) {
            if npc.hp_body.robot {
                set_anim(&mut an, &level, npc, "Disabled1", "Still", 1.0);
                continue;
            }
            set_anim(&mut an, &level, npc, "Shocked", "BreatheLight", 1.0);
            continue;
        }
        if let NpcState::RubbingEyes(t) = npc.state {
            let starting =
                npc.anim == "RubEyesStart" && an.clip.as_ref().map(|c| !c.done()).unwrap_or(false);
            if !starting && t > 0.0 {
                set_anim(&mut an, &level, npc, "RubEyes", "BreatheLight", 1.0);
            }
            continue;
        }
        let two = npc.two_handed();
        let melee_w = npc.weapon.as_ref().map_or(false, |w| w.hand_to_hand);
        // Pawns run normally with a knife (PlayRunningAndFiring with bHandToHand).
        let firing = firing && !(melee_w && speed > 20.0);
        let (walk_m, run_m) = (npc.walk_anim_mult, npc.run_anim_mult);
        if npc.sitting {
            set_anim(&mut an, &level, npc, "SitBreathe", "BreatheLight", 1.0);
        } else if npc.state == NpcState::Fleeing && npc.cower {
            // PlayCowering.
            set_anim(&mut an, &level, npc, "CowerStill", "BreatheLight", 1.0);
        } else if npc.state == NpcState::Burning {
            // PlayPanicRunning.
            set_anim(&mut an, &level, npc, "Panic", "Run", run_m);
        } else if speed > 20.0 {
            match (running, firing, two) {
                (true, true, true) => set_anim(&mut an, &level, npc, "RunShoot2H", "Run", run_m),
                (true, true, false) => set_anim(&mut an, &level, npc, "RunShoot", "Run", run_m),
                (true, false, true) => set_anim(&mut an, &level, npc, "RunShoot2H", "Run", run_m),
                (true, false, false) => set_anim(&mut an, &level, npc, "Run", "Walk", run_m),
                (false, _, true) => set_anim(&mut an, &level, npc, "Walk2H", "Walk", walk_m),
                (false, _, false) => set_anim(&mut an, &level, npc, "Walk", "", walk_m),
            }
        } else if firing && npc.reload_timer > 0.0 {
            // TweenToShoot while the weapon reloads plays the looping Reload instead.
            set_anim(&mut an, &level, npc, "Reload", "BreatheLight", 1.0);
        } else if firing {
            // In a fight the pawn never drops back to its idle pose: PlayShoot once per shot, and between shots it
            // holds the aim (TweenToShoot: the Shoot sequence's first frame).
            let name = if melee_w {
                "Attack"
            } else if npc.crouching {
                "CrouchShoot"
            } else if let Some(n) = crate::pawnclass::shoot_anim(&lvl.actors[npc.actor].class) {
                n
            } else if two {
                "Shoot2H"
            } else {
                "Shoot"
            };
            let ma = &lvl.mesh_actors[an.mesh_actor];
            let mesh = ma.anim.as_ref().map(|a| a.mesh.clone());
            if let Some(mesh) = mesh {
                let have = |n: &str| mesh.seqs.iter().any(|s| s.name.eq_ignore_ascii_case(n));
                let name = if have(name) {
                    name
                } else if melee_w && have("Shoot") {
                    "Shoot"
                } else {
                    name
                };
                if npc.just_fired {
                    if let Some(c) = Clip::named(&mesh, name, false) {
                        an.clip = Some(c);
                        npc.anim = "ShootOnce";
                    }
                } else {
                    let playing = npc.anim == "ShootOnce"
                        && an.clip.as_ref().map(|c| !c.done()).unwrap_or(false);
                    if !playing && npc.anim != "Aim" {
                        if let Some(mut c) = Clip::named(&mesh, name, false) {
                            c.rate = 0.0;
                            an.clip = Some(c);
                            npc.anim = "Aim";
                        }
                    }
                }
            }
        } else {
            // LoopBaseConvoAnim: a speaking pawn gestures now and then (every 2.5 s, 30%), at 0.35 rate.
            let gesturing =
                npc.anim == "Gesture" && an.clip.as_ref().is_some_and(|c| !c.done());
            if an.face.phoneme.is_some() && !gesturing {
                an.face.gesture_t += dt;
                if an.face.gesture_t > 2.5 {
                    an.face.gesture_t = 0.0;
                    let r = crate::sound::frand();
                    let g = if r < 0.1 {
                        Some("GestureLeft")
                    } else if r < 0.2 {
                        Some("GestureRight")
                    } else if r < 0.3 {
                        Some("GestureBoth")
                    } else {
                        None
                    };
                    let mesh = lvl.mesh_actors[an.mesh_actor]
                        .anim
                        .as_ref()
                        .map(|a| a.mesh.clone());
                    if let (Some(g), Some(mesh)) = (g, mesh) {
                        if let Some(mut c) = Clip::named(&mesh, g, false) {
                            c.rate *= 0.35;
                            an.clip = Some(c);
                            npc.anim = "Gesture";
                        }
                    }
                }
            }
            let gesturing =
                npc.anim == "Gesture" && an.clip.as_ref().is_some_and(|c| !c.done());
            if !gesturing {
                set_anim(
                    &mut an,
                    &level,
                    npc,
                    if two {
                        "BreatheLight2H"
                    } else {
                        "BreatheLight"
                    },
                    "BreatheLight",
                    1.0,
                );
            }
        }
    }
    pawn_hits.0.extend(new_hits);
    noises.0.extend(new_noises);
}

/// Health readout, red damage flash, and restart after death.
#[allow(clippy::too_many_arguments)]
pub fn player_health_ui(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<LevelRes>,
    mut ph: ResMut<PlayerHealth>,
    mut player: ResMut<Player>,
    mut hud: ResMut<HudMessage>,
    mut text: Single<&mut Text, With<HealthText>>,
    mut flash: Single<&mut BackgroundColor, With<DamageFlash>>,
) {
    let _prof = crate::prof::scope("ai::player_health_ui");
    let dt = time.delta_secs();
    ph.flash = (ph.flash - dt).max(0.0);
    ph.drug_timer = (ph.drug_timer - dt).max(0.0);
    // UpdatePoison.
    if ph.poison_counter > 0 && !ph.dead {
        ph.poison_timer += dt;
        if ph.poison_timer >= 2.0 {
            ph.poison_timer = 0.0;
            ph.poison_counter -= 1;
            let d = ph.poison_damage;
            ph.damage(d, "PoisonEffect", DVec3::ZERO);
        }
        if ph.poison_counter == 0 {
            ph.poison_damage = 0.0;
        }
    }
    if ph.dead {
        // Ours: Enter (pad: the Jump button) loads the most recent save; worded for the device in use.
        let f = crate::gamepad::frame();
        let what = if crate::save::latest().is_some() { "load your last save" } else { "restart" };
        let t = if f.connected && f.pad_last {
            let k = crate::gamepad::glyph_kind();
            let name = |a: &str, d: crate::gamepad::PadBtn| crate::gamepad::pad_name(k, crate::gamepad::button_for(a).unwrap_or(d));
            format!(
                "You are dead. Press {} or {} for the menu, {} to {what}.",
                crate::gamepad::pad_name(k, crate::gamepad::PadBtn::Start),
                name("Fire / throw", crate::gamepad::PadBtn::RT),
                name("Jump / swim up", crate::gamepad::PadBtn::South),
            )
        } else {
            format!("You are dead. Press Escape or Fire for the menu, Enter to {what}.")
        };
        if hud.text != t || hud.time_left < 1.0e8 {
            hud.text = t;
            hud.time_left = 1.0e9;
        }
    }
    if !ph.dead {
        // Gas and other coloured flashes are a light tint (ClientFlash fog of 0.05-0.15); plain hits keep the red.
        let cap = if ph.flash_color == [0.6, 0.0, 0.0] {
            0.5
        } else {
            0.18
        };
        flash.0 = Color::srgba(
            ph.flash_color[0],
            ph.flash_color[1],
            ph.flash_color[2],
            (ph.flash * 1.2).min(cap),
        );
    }
    // Health, energy and breath are on the original hit display (classic_hud.rs); charged items stay here until
    // the active items display is ported.
    let t = ph.charged_text.clone();
    if text.0 != t {
        text.0 = t;
    }
    let pad_confirm = crate::gamepad::frame().pad_last && keys.just_pressed(KeyCode::Space);
    if ph.dead && !crate::menus::open() && (keys.just_pressed(KeyCode::Enter) || pad_confirm) {
        // The most recent save (manual, quick or auto) is loaded; with none, the map is rebuilt from scratch in
        // place (loader.rs) with the same character: every pawn, door, item and flag back to how the map starts.
        if let Some(last) = crate::save::latest() {
            let root = std::path::PathBuf::new();
            crate::save::load(&root, &last);
            return;
        }
        if std::env::var("DXVIEW_SOFT_RESTART").is_err() {
            let map = level.0.name.clone();
            crate::loader::request(&map, crate::loader::Goal::Restart);
            return;
        }
        *ph = PlayerHealth::default();
        hud.text.clear();
        hud.time_left = 0.0;
        if let Some(ps) = &level.0.player_start {
            let s = dx_level::bevy_to_ue(ps.position);
            player.body = Character::new(DVec3::new(s[0] as f64, s[1] as f64, s[2] as f64));
            player.body.physics = Physics::Falling;
        }
        // Respawn always puts you back on foot, even if you died while flying.
        player.noclip = false;
    }
}

pub fn spawn_health_ui(commands: &mut Commands) {
    commands.spawn((
        Text::new("HEALTH 100"),
        TextFont {
            font_size: bevy::text::FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 1.0, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(16.0),
            top: Val::Px(16.0),
            ..default()
        },
        HealthText,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.6, 0.0, 0.0, 0.0)),
        DamageFlash,
        Pickable::IGNORE,
    ));
}

/// The weapon a pawn holds, drawn in its hand.
#[derive(Component)]
pub struct HeldModel {
    pub root: Entity,
    pub flash: Option<Entity>,
    pub parts: Vec<dx_level::actors::MeshPart>,
    pub handles: Vec<Handle<Mesh>>,
    pub scale: f32,
    pub relight: f32,
    /// Where it was last lit (Unreal units) and whether a dynamic light reached it then: a weapon that hasn't
    /// moved and has no dynamic light near is not re-lit (re-uploading its mesh for nothing).
    pub lit_at: Option<([f32; 3], bool)>,
    /// dynlight::light_sig of the lights it was last lit with.
    pub lit_sig: Option<u64>,
    /// Weapon class this model shows; a different weapon in hand rebuilds it.
    pub class: String,
}

/// Fleeing.PickDestination: up to three HidePoints the enemy can't see, scored by how directly they lead away
/// from the enemy (4 - |toEnemy + toPoint|^2) and picked at random weighted by score; failing that, a spot about
/// 400 units away roughly opposite the enemy that we can get to in a straight line.
fn pick_flee_dest(
    lvl: &dx_level::Level,
    me: DVec3,
    enemy: DVec3,
    radius: f64,
    r: f32,
    r2: f32,
) -> DVec3 {
    let mut cands: Vec<(f64, f64, DVec3)> = Vec::new();
    let eye = enemy + DVec3::new(0.0, 0.0, 30.0);
    for p in lvl
        .nav
        .points
        .iter()
        .filter(|p| p.class.eq_ignore_ascii_case("HidePoint"))
    {
        let pos = DVec3::from(p.pos);
        let dist = pos.distance(me);
        if dist > 10000.0 || lvl.collision.trace(eye, pos, DVec3::ZERO).fraction >= 0.999 {
            continue;
        }
        let v1 = (enemy - me).normalize_or_zero();
        let v2 = (pos - me).normalize_or_zero();
        let score = 4.0 - (v1 + v2).length_squared();
        cands.push((score, dist, pos));
    }
    cands.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    cands.truncate(3);
    let total: f64 = cands.iter().map(|c| c.0.max(0.0)).sum();
    if total > 0.0 {
        let mut x = r as f64 * total;
        for c in &cands {
            x -= c.0.max(0.0);
            if x <= 0.0 {
                return c.2;
            }
        }
        return cands[0].2;
    }
    let away = (me - enemy).truncate();
    let base = away.y.atan2(away.x);
    let mag = 400.0 * (r2 as f64 * 0.4 + 0.8);
    let ext = DVec3::new(radius, radius, 20.0);
    for i in 0..8 {
        let off = (i as f64 * 0.37 + r as f64).fract() - 0.5;
        let yaw = base + off * std::f64::consts::PI * 0.6;
        let d = DVec3::new(yaw.cos(), yaw.sin(), 0.0);
        let h = lvl.collision.trace(me, me + d * mag, ext);
        if h.fraction > 0.5 {
            return me + d * mag * (h.fraction - 0.05).max(0.0);
        }
    }
    me + away.extend(0.0).normalize_or_zero() * 300.0
}

/// Weapons pawns dropped (class, where): spawned as pickups.
#[derive(Resource, Default)]
pub struct WeaponDrops(pub Vec<(String, DVec3)>);

/// Lit flares thrown by the player (Flare.LightFlare): where from and how fast.
#[derive(Resource, Default)]
pub struct FlareThrows(pub Vec<(DVec3, DVec3)>);

/// Unreal space to Bevy space as a matrix (axis swap and unit scale), and the axis swap alone.
fn swap() -> Mat3 {
    Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::Y)
}

/// Give armed pawns a weapon model and keep it on the hand's attachment triangle every frame.
#[allow(clippy::too_many_arguments)]
pub fn npc_weapon_models(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    new: Query<(Entity, &Npc), Without<HeldModel>>,
    mut held: Query<(Entity, &Npc, &Animated, &Transform, &mut HeldModel), Without<HeldRoot>>,
    mut roots: Query<(&mut Transform, &mut Visibility), With<HeldRoot>>,
    (mut flashes, mut shells, mut seed, blockers): (
        Query<&mut crate::weapons::MuzzleFlashPart>,
        ResMut<crate::destruct::Shells>,
        Local<u64>,
        Res<Blockers>,
    ),
) {
    let _prof = crate::prof::scope("ai::npc_weapon_models");
    for (e, npc) in &new {
        let Some(w) = npc.weapon.as_ref() else {
            continue;
        };
        let Some(mesh) = w.third_mesh.clone() else {
            continue;
        };
        let mut d = assets.classes.defaults(&w.class);
        // Augmentique: a skin for this owner's weapon (WeaponSkinManager.SetDefaultSkin).
        let npc_class = level.0.actors.get(npc.actor).map(|a| a.class.clone()).unwrap_or_default();
        crate::augmentique::apply_weapon_skin(&mut d, &w.class, &crate::augmentique::npc_skin(&npc_class, &w.class), false);
        let root = commands
            .spawn((Transform::default(), Visibility::Hidden, HeldRoot))
            .id();
        commands.entity(e).add_child(root);
        let map = |p: [f32; 3]| ue_to_bevy(p);
        if let Some((_m, parts, handles)) = spawn_mesh_parts(
            &mesh,
            1.0,
            &d,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut commands,
            root,
            &map,
            None,
        ) {
            let flash = if w.muzzle_flash {
                crate::weapons::add_muzzle_flash(
                    &mut commands,
                    &mut assets,
                    &mut images,
                    &mut materials,
                    root,
                    &parts,
                    &handles,
                    None,
                    None,
                )
            } else {
                None
            };
            commands.entity(e).insert(HeldModel {
                root,
                flash,
                parts,
                handles,
                scale: w.third_scale,
                relight: 0.0,
                lit_at: None,
                lit_sig: None,
                class: w.class.clone(),
            });
        } else {
            commands.entity(root).despawn();
            // Remember that this pawn has nothing drawable so we stop trying.
            commands.entity(e).insert(HeldModel {
                root: Entity::PLACEHOLDER,
                flash: None,
                parts: Vec::new(),
                handles: Vec::new(),
                scale: 1.0,
                relight: 1.0e9,
                lit_at: None,
                lit_sig: None,
                class: w.class.clone(),
            });
        }
    }
    let dt = time.delta_secs();
    for (e, npc, an, tf, mut hm) in &mut held {
        // Switched weapons (SwitchToBestWeapon): drop this model; a new one is made next frame.
        if npc.weapon.as_ref().map(|w| w.class.as_str()) != Some(hm.class.as_str()) {
            if hm.root != Entity::PLACEHOLDER {
                commands.entity(hm.root).despawn();
            }
            commands.entity(e).remove::<HeldModel>();
            continue;
        }
        if npc.just_fired {
            if let Some(mut f) = hm.flash.and_then(|e| flashes.get_mut(e).ok()) {
                f.left = 0.1;
            }
            // MuzzleFlashLight: a MuzzleFlash light at Location + view x CollisionRadius x 2 for 0.1 s.
            if npc.weapon.as_ref().map_or(false, |w| w.muzzle_flash) {
                let at = dx_level::bevy_to_ue(tf.translation.to_array());
                let c = DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64);
                let fwd = DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0);
                crate::fx::muzzle(c + fwd * (npc.params.radius + 24.0) + DVec3::Z * npc.params.height * 0.4, fwd);
                crate::dynlight::flash(
                    c + DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0) * npc.params.radius * 2.0,
                    crate::dynlight::MUZZLE_FLASH,
                    0.1,
                );
            }
            if let Some(w) = npc.weapon.as_ref() {
                if let Some(class) = crate::destruct::shell_for(
                    npc.ammo.as_deref(),
                    w.silenced || w.noise_level < 0.1,
                ) {
                    if *seed == 0 {
                        *seed = 0xACE5;
                    }
                    // Owner.Location is the collision centre (the drawn mesh can sit off it by PrePivot).
                    let c = match blockers.list.get(npc.blocker) {
                        Some(b) => b.center,
                        None => {
                            let at = dx_level::bevy_to_ue(tf.translation.to_array());
                            DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64)
                        }
                    };
                    let (pos, vel) = crate::destruct::shell_launch(
                        c,
                        npc.params.radius,
                        npc.params.height,
                        DVec3::new(npc.yaw.cos(), npc.yaw.sin(), 0.0),
                        &mut seed,
                    );
                    shells.0.push((class, pos, vel));
                }
            }
        }
        let Ok((mut rtf, mut vis)) = roots.get_mut(hm.root) else {
            continue;
        };
        let ma = &level.0.mesh_actors[an.mesh_actor];
        let Some(anim) = &ma.anim else { continue };
        let (start, frames, f) = crate::actors::current_frame(an, anim);
        let coords = if npc.state == NpcState::Dead || !npc.weapon_drawn {
            None
        } else {
            anim.mesh.weapon_coords(start, frames, f)
        };
        let Some([o, x, y, z]) = coords else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Inherited;
        // Pawn-local Unreal: p' = O * DrawScale + [X Y Z] * ThirdPersonScale * p; then into Bevy axes.
        let ds = anim.draw_scale;
        let r = Mat3::from_cols(Vec3::from(x), Vec3::from(y), Vec3::from(z)) * hm.scale;
        let s = swap();
        let lin = s * r * s;
        let t = Vec3::from(ue_to_bevy([o[0] * ds, o[1] * ds, o[2] * ds]));
        *rtf = Transform::from_matrix(Mat4::from_cols(
            lin.x_axis.extend(0.0),
            lin.y_axis.extend(0.0),
            lin.z_axis.extend(0.0),
            t.extend(1.0),
        ));
        // Light it from the static lights near the pawn now and then.
        hm.relight -= dt;
        if hm.relight <= 0.0 {
            hm.relight = 0.5;
            let at = dx_level::bevy_to_ue(tf.translation.to_array());
            let d2 = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
            let dyn_near = crate::dynlight::current().iter().any(|l| d2(l.pos, at) < (l.radius + 96.0).powi(2));
            let stale = match hm.lit_at {
                None => true,
                Some((p, was_dyn)) => dyn_near || was_dyn || d2(p, at) > 8.0 * 8.0,
            };
            if !stale {
                continue;
            }
            hm.lit_at = Some((at, dyn_near));
            let lit_by = crate::dynlight::lights_at(&level.0, at, 24.0);
            let sig = crate::dynlight::light_sig(&lit_by, at, 0);
            if hm.lit_sig == Some(sig) {
                continue;
            }
            hm.lit_sig = Some(sig);
            for (pi, h) in hm.handles.iter().enumerate() {
                if let Some(mut m) = meshes.get_mut(h) {
                    let c = dx_level::actors::light_part(&hm.parts[pi], at, [0, 0, 0], 1.0, &lit_by);
                    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
                }
            }
        }
    }
}

#[derive(Component)]
pub struct HeldRoot;

#[cfg(test)]
mod player_health_tests {
    use super::*;

    #[test]
    fn regions_and_spill() {
        let mut h = PlayerHealth::default();
        // Torso hit counts double.
        h.damage(10.0, "Generic", DVec3::new(0.0, 0.0, 10.0));
        assert_eq!(h.parts[P_TORSO], 80.0);
        // A dead leg passes the rest to the other leg.
        h.damage(130.0, "Generic", DVec3::new(0.0, 5.0, -10.0));
        assert_eq!(h.parts[P_LEG_R], 0.0);
        assert_eq!(h.parts[P_LEG_L], 70.0);
        assert!(!h.dead);
        assert_eq!(h.speed_factor().1, false);
    }

    #[test]
    fn total_health_and_head() {
        let mut h = PlayerHealth::default();
        h.damage(50.0, "Generic", DVec3::new(0.0, 0.0, PLAYER_HEIGHT * 0.9));
        assert_eq!(h.parts[P_HEAD], 0.0);
        assert!(h.dead);
        let mut h = PlayerHealth::default();
        h.damage(10.0, "Shot", DVec3::new(0.0, PLAYER_RADIUS, 0.0));
        assert_eq!(h.parts[P_ARM_R], 85.0);
        assert!(h.accuracy_penalty() == 0.0);
    }
}

#[cfg(test)]
mod weapon_choice_tests {
    use super::*;

    fn stats(class: &str, reload: u32, acc: f32, melee: bool, ammo: Option<&str>) -> WeaponStats {
        let mut p = vec![
            Property {
                name: "PlayerViewMesh".into(),
                index: 0,
                value: Value::Path("X.Y".into()),
            },
            Property {
                name: "ReloadCount".into(),
                index: 0,
                value: Value::Int(reload as i32),
            },
            Property {
                name: "AccurateRange".into(),
                index: 0,
                value: Value::Int(acc as i32),
            },
            Property {
                name: "maxRange".into(),
                index: 0,
                value: Value::Int(acc as i32 * 2),
            },
            Property {
                name: "bInstantHit".into(),
                index: 0,
                value: Value::Bool(true),
            },
        ];
        if melee {
            p.push(Property {
                name: "bHandToHand".into(),
                index: 0,
                value: Value::Bool(true),
            });
            p.push(Property {
                name: "bFallbackWeapon".into(),
                index: 0,
                value: Value::Bool(true),
            });
        }
        if let Some(a) = ammo {
            p.push(Property {
                name: "AmmoName".into(),
                index: 0,
                value: Value::Path(format!("DeusEx.{a}")),
            });
        }
        weapon_stats(class, &p).unwrap()
    }

    #[test]
    fn knife_only_when_the_gun_is_empty() {
        let mut n = Npc::new(0, 0, &[], 0, DVec3::ZERO);
        n.arsenal = vec![
            stats("WeaponPistol", 6, 2400.0, false, Some("Ammo10mm")),
            stats("WeaponCombatKnife", 0, 80.0, true, None),
        ];
        n.ammo = Some("Ammo10mm".into());
        n.ammo_left = vec![("Ammo10mm".into(), 2)];
        // Even point blank, the gun beats the fallback knife while it has ammo.
        n.switch_to_best_weapon(Some((40.0, 16.0, false, true)));
        assert_eq!(n.weapon.as_ref().unwrap().class, "WeaponPistol");
        n.use_ammo();
        n.use_ammo();
        assert_eq!(n.weapon_ammo(), Some(0));
        n.switch_to_best_weapon(Some((40.0, 16.0, false, true)));
        assert_eq!(n.weapon.as_ref().unwrap().class, "WeaponCombatKnife");
    }

    #[test]
    fn karkian_rams_then_bites_while_fire_timer_runs() {
        let mut n = Npc::new(0, 0, &[], 0, DVec3::ZERO);
        let mut bump = stats("WeaponKarkianBump", 0, 80.0, true, None);
        bump.fallback = false;
        bump.ai_fire_delay = 8.0;
        let mut bite = stats("WeaponKarkianBite", 0, 120.0, true, None);
        bite.fallback = false;
        n.arsenal = vec![bite, bump];
        n.weapon_timer = 100.0;
        n.fire_timer = 8.0;
        n.switch_to_best_weapon(Some((60.0, 16.0, false, true)));
        assert_eq!(n.weapon.as_ref().unwrap().class, "WeaponKarkianBite");
    }

    #[test]
    fn spawned_hostile_animals_hate_the_player() {
        for c in ["Doberman", "Greasel", "Karkian", "KarkianBaby", "Gray"] {
            let p = crate::spawn::hostile_animal_alliance(c).unwrap();
            let n = Npc::new(
                0,
                0,
                &[
                    p,
                    Property {
                        name: "Alliance".into(),
                        index: 0,
                        value: Value::Name(c.into()),
                    },
                ],
                0,
                DVec3::ZERO,
            );
            assert!(n.hates("Player"), "{c}");
        }
        for c in ["Mutt", "Cat", "Rat"] {
            assert!(crate::spawn::hostile_animal_alliance(c).is_none());
        }
    }
}

/// Enhancement: bodies (dead or unconscious) found by guards raise the alarm (setting BodyAlarms).
pub fn body_alarms() -> bool {
    crate::settings::get("BodyAlarms") > 0.5
}

fn me_center_of(blockers: &Blockers, b: usize) -> DVec3 {
    blockers.list.get(b).map(|c| c.center).unwrap_or(DVec3::ZERO)
}

/// Pawns killed without a death animation (Animal.bPlayDying False): hide the pawn and drop its carcass mesh
/// (CarcassType's Mesh) where it was.
#[allow(clippy::too_many_arguments)]
pub fn carcass_swap_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut q: Query<(&mut Npc, &Transform, &mut Visibility)>,
    blockers: Res<Blockers>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("ai::carcass_swap_system");
    let dt = time.delta_secs();
    for (mut npc, _tf, mut vis) in &mut q {
        if npc.carried {
            if let Some(e) = npc.float_body.take() {
                commands.entity(e).despawn();
            }
        }
        // DeusExCarcass in a water zone: Mesh3, the floating body, at the surface; still searchable.
        if let Some((class, t)) = npc.water_carcass.as_mut() {
            *t -= dt;
            if *t > 0.0 {
                continue;
            }
            let class = class.clone();
            npc.water_carcass = None;
            if npc.carried {
                continue;
            }
            let d = assets.classes.defaults(&class);
            let Some(Value::Path(mesh)) = find(&d, "Mesh3").cloned() else { continue };
            *vis = Visibility::Hidden;
            let mut at = npc.body.as_ref().map(|b| b.location).unwrap_or_else(|| blockers.list[npc.blocker].center);
            // Up to the surface.
            for _ in 0..64 {
                if crate::actors::water_at(&level.0, at + DVec3::new(0.0, 0.0, 4.0)).is_none() {
                    break;
                }
                at.z += 4.0;
            }
            let mut loose = crate::destruct::Loose::dropped(at, 4.0);
            loose.resting = true;
            loose.rot = [0.0, npc.yaw / std::f64::consts::TAU * 65536.0, 0.0];
            if let Some(e) = crate::destruct::spawn_loose(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, &level.0, &mesh, &d, crate::sound::fprop(&d, "DrawScale", 1.0), loose) {
                npc.float_body = Some(e);
            }
            continue;
        }
        let Some(class) = npc.swap_carcass.take() else {
            continue;
        };
        *vis = Visibility::Hidden;
        npc.carried = true;
        npc.searched = true;
        let at = blockers.list[npc.blocker].center;
        let d = assets.classes.defaults(&class);
        let Some(Value::Path(mesh)) = find(&d, "Mesh").cloned() else {
            continue;
        };
        let h = find(&d, "CollisionHeight")
            .and_then(|v| {
                if let Value::Float(f) = v {
                    Some(*f as f64)
                } else {
                    None
                }
            })
            .unwrap_or(4.0);
        let loose = crate::destruct::Loose::dropped(at, h);
        crate::destruct::spawn_loose(
            &mut commands,
            &mut assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &level.0,
            &mesh,
            &d,
            crate::sound::fprop(&d, "DrawScale", 1.0),
            loose,
        );
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
/// ScriptedPawn.IsDoor: a DeusExMover with more than one keyframe and bIsDoor.
fn is_door(lvl: &dx_level::Level, i: usize) -> bool {
    lvl.movers[i].num_keys > 1 && lvl.movers[i].is_door
}

/// ScriptedPawn.FrobDoor: a decoration whose Event is the door's Tag (its button) triggers every mover with that tag;
/// otherwise a frobbable door is frobbed, unless another pawn is already waiting on it. False = nothing done.
fn frob_door(lvl: &dx_level::Level, mv: &Movers, i: usize, me: usize) -> bool {
    let info = &lvl.movers[i];
    let tag = info.tag.clone();
    let button = !tag.is_empty()
        && lvl.actors.iter().any(|a| {
            find(&a.props, "bPushable").is_some() && crate::devices::name_prop(&a.props, "Event").eq_ignore_ascii_case(&tag)
        });
    if button {
        crate::movers::pawn_door(crate::movers::PawnDoor::Trigger(tag, me));
        return true;
    }
    let rt = &mv.rt[i];
    if rt.frobbable.unwrap_or(info.frobbable) && rt.waiting_pawn.map_or(true, |w| w == me) {
        crate::movers::pawn_door(crate::movers::PawnDoor::Frob(i, me));
        return true;
    }
    false
}

/// OpeningDoor.FindBackupPoint: inside the door's movement area (grown by our radius + 30), back away along the
/// normal of the door face we walked into until we're out of it (45..700 units) if that way is clear; else stay.
/// The area here is the circle the door sweeps around its pivot.
fn backup_point(lvl: &dx_level::Level, mv: &Movers, i: usize, at: DVec3, normal: DVec3, radius: f64, height: f64) -> DVec3 {
    let info = &lvl.movers[i];
    let reach = info
        .parts
        .iter()
        .flat_map(|p| p.positions.iter())
        .map(|p| (p[0] as f64).hypot(p[1] as f64))
        .fold(0.0f64, f64::max)
        + radius
        + 30.0;
    let c = mv.rt[i].location;
    let n = DVec3::new(normal.x, normal.y, 0.0).normalize_or_zero();
    if n == DVec3::ZERO {
        return at;
    }
    let rel = DVec3::new(at.x - c.x, at.y - c.y, 0.0);
    if rel.length() >= reach {
        return at;
    }
    // |rel + n t| = reach, the root ahead of us.
    let b = rel.dot(n);
    let t = -b + (b * b - (rel.length_squared() - reach * reach)).max(0.0).sqrt();
    let t = t.clamp(45.0, 700.0);
    let to = at + n * t;
    let ext = DVec3::new(radius, radius, height);
    if lvl.collision.trace(at, to, ext).fraction < 1.0 {
        return at;
    }
    to
}

pub fn reset_level() {
    if let Ok(mut v) = WORLD.lock() {
        v.clear();
    }
    if let Ok(mut v) = TAKEN.lock() {
        v.clear();
    }
}
