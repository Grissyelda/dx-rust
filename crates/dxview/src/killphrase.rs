//! Killphrases (MULTIPLAYER roadmap Phase 3, Modern; the room's "Killphrases" switch, off unless the host turns it on).
//!
//! Every player has a phrase of three words for as long as he lives (a new one each life: counter "new_phrase"). Its
//! words are scattered as fragments:
//! - datacubes the map gets when the match has killphrases (placed beside some of its kept pickups, `kp_cubes` of
//!   them), each holding one word of one player's phrase;
//! - an intercepted message at each Overwatch seat (overwatch.rs);
//! - interrogating a downed rival (downed.rs) gives his next word.
//! Knowing all three words of a rival, Use on him within `kp_reach` says the phrase to his face and he dies at once.
//!
//! The host draws the phrases (from the data's word list) and decides which fragment each cube and seat holds; every
//! machine knows the table, and shows only what its player has found (and keeps that across his own deaths: what he
//! learnt of a rival stays good until that rival dies).

use crate::mech::{self, MechMsg};
use bevy::prelude::*;
use std::sync::Mutex;

/// Spots at or past this hold seat mail (100 + the seat's place among the map's security computers).
const SEAT_SPOT: u16 = 100;

#[derive(Default)]
struct State {
    phrases: Vec<(u8, [String; 3])>,
    spots: Vec<(u16, u8, u8)>,
    /// Fragments found: (whose, which word, the word).
    known: Vec<(u8, u8, String)>,
    seats_read: Vec<usize>,
    /// Host: the players the table was made for, a counter for new phrases, the resend clock.
    for_players: Vec<u8>,
    seed: u64,
    send_t: f32,
    /// The phrase was said to us by this player (applied by killphrase_system).
    spoken_by: Option<u8>,
    speak_t: f32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn enabled() -> bool {
    mech::on(|r| r.killphrases)
}

fn name_of(id: u8) -> String {
    if id == crate::deathmatch::my_id() {
        crate::travel::player_name()
    } else {
        crate::netplay::players().into_iter().find(|p| p.0 == id).map_or_else(|| format!("Player {id}"), |p| p.1)
    }
}

fn next(seed: &mut u64) -> u64 {
    // splitmix64
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Three different words from the list.
fn draw_phrase(seed: &mut u64) -> [String; 3] {
    let words = &mech::tuning().kp_words;
    if words.len() < 3 {
        return ["alpha".into(), "bravo".into(), "charlie".into()];
    }
    let mut picks: Vec<usize> = Vec::new();
    while picks.len() < 3 {
        let i = (next(seed) % words.len() as u64) as usize;
        if !picks.contains(&i) {
            picks.push(i);
        }
    }
    [words[picks[0]].clone(), words[picks[1]].clone(), words[picks[2]].clone()]
}

/// Which fragment each spot holds: spot k goes round the players and their words.
fn assign(players: &[u8], cubes: usize, seats: usize) -> Vec<(u16, u8, u8)> {
    if players.is_empty() {
        return Vec::new();
    }
    let n = players.len();
    let mut out = Vec::new();
    for k in 0..cubes {
        out.push((k as u16, players[k % n], ((k / n) % 3) as u8));
    }
    // Seats hold the words the cubes reach last.
    for k in 0..seats {
        let j = k + cubes;
        out.push((SEAT_SPOT + k as u16, players[j % n], ((j / n) % 3) as u8));
    }
    out
}

/// The host's table.
pub fn set_phrases(p: Vec<(u8, [String; 3])>, spots: Vec<(u16, u8, u8)>) {
    with(|s| {
        // What we learnt of a rival whose phrase changed is worthless now.
        let phrases = p.clone();
        s.known.retain(|(of, w, word)| phrases.iter().any(|(id, ph)| id == of && ph.get(*w as usize) == Some(word)));
        s.phrases = p;
        s.spots = spots;
    });
}

/// Host: a player died - his next life gets a new phrase.
pub fn host_died(victim: u8) {
    if !enabled() {
        return;
    }
    with(|s| {
        if s.seed == 0 {
            s.seed = 0x5EED_0F_1A2B;
        }
        let mut seed = s.seed;
        let fresh = draw_phrase(&mut seed);
        s.seed = seed;
        if let Some(p) = s.phrases.iter_mut().find(|p| p.0 == victim) {
            p.1 = fresh;
        }
        s.send_t = 0.0;
    });
}

/// A fragment found: whose phrase, which word.
pub fn learn(of: u8, w: u8, from: &str) -> Option<String> {
    let word = with(|s| s.phrases.iter().find(|p| p.0 == of).and_then(|p| p.1.get(w as usize).cloned()))?;
    let new = with(|s| {
        if s.known.iter().any(|k| k.0 == of && k.1 == w) {
            false
        } else {
            s.known.push((of, w, word.clone()));
            true
        }
    });
    let line = format!("{}'s killphrase, word {} of 3: \"{}\" ({from}).", name_of(of), w + 1, word.to_uppercase());
    if new {
        println!("KILLPHRASE learnt {line}");
        crate::classic_hud::client_message(&line);
        crate::consys::add_vault_note(&format!("Killphrase fragment - {line}"));
        crate::sound::play_player("DeusExSounds.UserInterface.LogNoteAdded", 0.8, 0.0);
        if complete(of) {
            crate::classic_hud::client_message(&format!("You know {}'s whole killphrase. Say it to his face (Use on him).", name_of(of)));
        }
    }
    Some(line)
}

/// Interrogation (downed.rs): the next word of his phrase we don't know.
pub fn learn_next(of: u8) {
    if !enabled() {
        return;
    }
    let w = (0..3u8).find(|w| !with(|s| s.known.iter().any(|k| k.0 == of && k.1 == *w)));
    if let Some(w) = w {
        learn(of, w, "interrogation");
    }
}

fn complete(of: u8) -> bool {
    with(|s| (0..3u8).all(|w| s.known.iter().any(|k| k.0 == of && k.1 == w)))
}

/// Use on a rival with his whole phrase known.
pub fn can_speak(id: u8) -> bool {
    enabled() && complete(id)
}

/// pickups: say the phrase to rival `id`.
pub fn speak(id: u8) {
    let words: Vec<String> = with(|s| {
        let mut k: Vec<(u8, String)> = s.known.iter().filter(|k| k.0 == id).map(|k| (k.1, k.2.clone())).collect();
        k.sort();
        k.into_iter().map(|x| x.1.to_uppercase()).collect()
    });
    if with(|s| s.speak_t > 0.0) {
        return;
    }
    with(|s| s.speak_t = 2.0);
    crate::classic_hud::client_message(&format!("You say: \"{}\"", words.join(" ")));
    mech::to_host(MechMsg::Speak(id));
}

/// Host: `from` says `id`'s phrase. Close enough: he dies.
pub fn host_speak(from: u8, id: u8) {
    if !enabled() || !mech::host() {
        return;
    }
    let me = crate::deathmatch::my_id();
    let pos = |p: u8| {
        if p == me {
            HOST_POS.lock().ok().map(|x| *x)
        } else {
            crate::netplay::remote_bodies().into_iter().find(|b| b.0 == p).map(|b| b.1)
        }
    };
    let (Some(a), Some(b)) = (pos(from), pos(id)) else { return };
    let reach = crate::counters::value("Killphrase", "Distance", "reach", mech::tuning().kp_reach) as f64;
    if a.distance(b) > reach {
        crate::netplay::tell(from, "Too far: he has to hear it.");
        return;
    }
    println!("KILLPHRASE {} spoke {}'s phrase", name_of(from), name_of(id));
    crate::netplay::host_news(&format!("{} spoke {}'s killphrase.", name_of(from), name_of(id)));
    mech::to_player(id, MechMsg::Spoken(from));
}

/// The phrase was said to our face.
pub fn spoken(by: u8) {
    with(|s| s.spoken_by = Some(by));
}

static HOST_POS: Mutex<glam::DVec3> = Mutex::new(glam::DVec3::ZERO);

/// A datacube of ours (level extras): its text when read.
pub fn cube_text(tag: &str) -> Option<String> {
    let k: u16 = tag.strip_prefix("KillphraseCube")?.parse().ok()?;
    let spot = with(|s| s.spots.iter().find(|x| x.0 == k).copied());
    let me = crate::deathmatch::my_id();
    Some(match spot {
        Some((_, of, _)) if of == me => "A fragment of your own killphrase. Somebody else will be looking for this one.".to_string(),
        Some((_, of, w)) => learn(of, w, "a datacube").unwrap_or_else(|| "The cube's memory is scrambled.".into()),
        None => "The cube's memory is scrambled.".into(),
    })
}

fn seat_index(seat: usize, lvl: &dx_level::Level) -> Option<usize> {
    lvl.actors.iter().enumerate().filter(|(_, a)| crate::overwatch::is_seat(&a.class)).position(|(i, _)| i == seat)
}

/// The seat's intercepted message, if it holds one we haven't read (a preview line).
pub fn seat_mail(seat: usize, lvl: &dx_level::Level) -> Option<String> {
    if !enabled() || with(|s| s.seats_read.contains(&seat)) {
        return None;
    }
    let k = SEAT_SPOT + seat_index(seat, lvl)? as u16;
    let (_, of, _) = with(|s| s.spots.iter().find(|x| x.0 == k).copied())?;
    (of != crate::deathmatch::my_id()).then(|| "1 intercepted message".to_string())
}

/// Read it: the fragment it holds.
pub fn read_seat_mail(seat: usize, lvl: &dx_level::Level) -> Option<String> {
    let k = SEAT_SPOT + seat_index(seat, lvl)? as u16;
    with(|s| s.seats_read.push(seat));
    let (_, of, w) = with(|s| s.spots.iter().find(|x| x.0 == k).copied())?;
    learn(of, w, "an intercepted message")
}

/// The HUD: rivals whose phrase we've started on.
pub fn hud_lines() -> Vec<(String, [u8; 3])> {
    if !enabled() {
        return Vec::new();
    }
    let me = crate::deathmatch::my_id();
    let (known, phrases) = with(|s| (s.known.clone(), s.phrases.clone()));
    phrases
        .iter()
        .filter(|(id, _)| *id != me && known.iter().any(|k| k.0 == *id))
        .map(|(id, _)| {
            let words: Vec<String> = (0..3u8).map(|w| known.iter().find(|k| k.0 == *id && k.1 == w).map_or_else(|| "???".into(), |k| k.2.to_uppercase())).collect();
            (format!("{}: {}", name_of(*id).to_uppercase(), words.join(" ")), if complete(*id) { [255, 120, 220] } else { [200, 160, 230] })
        })
        .collect()
}

pub fn reset_level() {
    with(|s| *s = State::default());
}

/// Host: the table follows the players in the match (and goes out every few seconds); everyone: a phrase said to
/// our face kills us.
pub fn killphrase_system(time: Res<Time>, mut ph: ResMut<crate::ai::PlayerHealth>, player: Res<crate::Player>, level: Res<crate::LevelRes>) {
    if !enabled() {
        return;
    }
    let dt = time.delta_secs();
    with(|s| s.speak_t = (s.speak_t - dt).max(0.0));
    if let Ok(mut p) = HOST_POS.lock() {
        *p = player.body.location;
    }
    if let Some(by) = with(|s| s.spoken_by.take()) {
        if !ph.dead {
            crate::netplay::note_hurt_by(by);
            crate::classic_hud::client_message(&format!("{} said your killphrase.", name_of(by)));
            ph.damage(10000.0, "Killphrase", glam::DVec3::new(0.0, 0.0, 30.0));
        }
    }
    if !mech::host() {
        return;
    }
    let mut ids: Vec<u8> = crate::netplay::players().into_iter().map(|p| p.0).collect();
    ids.push(crate::deathmatch::my_id());
    ids.sort();
    ids.dedup();
    let seats = level.0.actors.iter().filter(|a| crate::overwatch::is_seat(&a.class)).count();
    let send = with(|s| {
        let mut changed = false;
        if s.for_players != ids {
            if s.seed == 0 {
                s.seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
            }
            for id in &ids {
                if !s.phrases.iter().any(|p| p.0 == *id) {
                    let mut seed = s.seed;
                    let ph = draw_phrase(&mut seed);
                    s.seed = seed;
                    s.phrases.push((*id, ph));
                }
            }
            s.phrases.retain(|p| ids.contains(&p.0));
            s.spots = assign(&ids, mech::tuning().kp_cubes, seats);
            s.for_players = ids.clone();
            changed = true;
        }
        s.send_t -= dt;
        if changed || s.send_t <= 0.0 {
            s.send_t = 4.0;
            Some((s.phrases.clone(), s.spots.clone()))
        } else {
            None
        }
    });
    if let Some((p, spots)) = send {
        mech::to_joiners(MechMsg::Phrases(p, spots));
    }
}

/// Level extras (living.rs adds them as the map is read): the match's datacubes, beside some of the map's kept
/// pickups.
pub fn cube_extras(map: &str, actors: &[dx_level::actors::ActorInfo]) -> Vec<(String, String, Vec<unreal_pkg::props::Property>)> {
    use unreal_pkg::props::{Property, Value};
    let Some(r) = crate::deathmatch::rules() else { return Vec::new() };
    if !r.mode.versus() || r.classic() || !r.killphrases {
        return Vec::new();
    }
    let Some(entry) = crate::deathmatch::map_entry(map) else { return Vec::new() };
    let mut spots: Vec<&dx_level::actors::ActorInfo> = actors
        .iter()
        .filter(|a| matches!(a.class.as_str(), "BioelectricCell" | "Lockpick" | "Multitool" | "MedKit") && entry.keep.iter().any(|k| k.eq_ignore_ascii_case(&a.name)))
        .collect();
    spots.sort_by(|a, b| a.name.cmp(&b.name));
    let n = mech::tuning().kp_cubes.min(spots.len());
    let mut out = Vec::new();
    for k in 0..n {
        let a = spots[k * spots.len() / n.max(1)];
        let l = a.location();
        let name = format!("KillphraseCube{k}");
        let props = vec![
            Property { name: "Location".into(), index: 0, value: Value::Vector([l[0] + 14.0, l[1] + 10.0, l[2]]) },
            Property { name: "Tag".into(), index: 0, value: Value::Name(name.clone()) },
            Property { name: "textTag".into(), index: 0, value: Value::Name(name.clone()) },
        ];
        out.push(("DataCube".to_string(), name, props));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_go_round_players_and_words() {
        let a = assign(&[0, 3, 5], 6, 2);
        assert_eq!(a[0], (0, 0, 0));
        assert_eq!(a[1], (1, 3, 0));
        assert_eq!(a[3], (3, 0, 1));
        assert_eq!(a[6], (SEAT_SPOT, 0, 2));
        let mut seed = 7;
        let p = draw_phrase(&mut seed);
        assert!(p[0] != p[1] && p[1] != p[2] && p[0] != p[2]);
    }
}
