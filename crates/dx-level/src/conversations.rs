//! Conversations (ConSys): the Conversation / ConEvent* / ConChoice / ConFlagRef objects in DeusExConText.u, as
//! plain data. Every object is an ordinary tagged property list; the event chain is followed through `nextEvent`,
//! choices through `nextChoice`, flag lists through `nextFlagRef`. ConEventRandomLabel keeps its labels in a native
//! dynamic array of strings after the properties. Speech audio lives in DeusExConAudio<audioPackageName>.u: a
//! ConAudioList whose dynamic array maps a speech's soundID to a Sound object.

use std::collections::HashMap;
use std::path::Path;
use unreal_pkg::props::{read_properties, skip_state_frame, Property, Value, RF_HAS_STACK};
use unreal_pkg::{Package, Reader};

#[derive(Debug, Clone, PartialEq)]
pub struct FlagRef {
    pub name: String,
    pub value: bool,
    /// Mission the flag expires in (0 = never).
    pub expiration: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub text: String,
    pub label: String,
    pub display_as_speech: bool,
    pub sound_id: i32,
    /// Skill class and level needed to see it.
    pub skill: Option<(String, i32)>,
    pub flags: Vec<FlagRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    Speech { speaker: String, speaking_to: String, text: String, sound_id: i32, continued: bool, bold: bool, computer_font: bool },
    Choice { clear_screen: bool, choices: Vec<Choice> },
    SetFlag(Vec<FlagRef>),
    CheckFlag { flags: Vec<FlagRef>, label: String },
    /// Object class (or NK_<key> name) to look for; jump to `fail_label` when missing.
    CheckObject { object_name: String, class: Option<String>, fail_label: String },
    TransferObject { object_name: String, class: Option<String>, count: i32, from: String, to: String, fail_label: String },
    /// CT_Predefined (position), CT_Speakers (rotation, height/center/distance), CT_Actor (offset, rotation),
    /// CT_Random.
    MoveCamera {
        camera_type: u8,
        position: u8,
        transition: u8,
        actor: String,
        offset: [f32; 3],
        rotation: [i32; 3],
        height: f32,
        center: f32,
        distance: f32,
    },
    Animation { owner: String, sequence: String, finish: bool, looped: bool },
    Trade { owner: String },
    /// Jump to a label, in another conversation when `con` is set (index into `ConDb::cons`).
    Jump { label: String, con: Option<usize> },
    Random { labels: Vec<String>, cycle: bool, cycle_once: bool, cycle_random: bool },
    Trigger { tag: String },
    AddGoal { name: String, completed: bool, text: String, primary: bool },
    AddNote { text: String },
    AddSkillPoints { points: i32, message: String },
    AddCredits(i32),
    /// EP_Credits / EP_Health / EP_SkillPoints compared with EC_Less.. EC_Greater against `value`.
    CheckPersona { persona: u8, condition: u8, value: i32, label: String },
    Comment(String),
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub label: String,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    pub name: String,
    pub description: String,
    pub owner_name: String,
    pub datalink: bool,
    pub display_once: bool,
    pub first_person: bool,
    pub non_interactive: bool,
    pub random_camera: bool,
    pub can_be_interrupted: bool,
    pub cannot_be_interrupted: bool,
    pub invoke_bump: bool,
    pub invoke_frob: bool,
    pub invoke_sight: bool,
    pub invoke_radius: bool,
    pub radius: i32,
    pub events: Vec<Event>,
    /// All must match for the conversation to be eligible.
    pub flags: Vec<FlagRef>,
    pub id: i32,
    pub audio_package: String,
}

impl Conversation {
    pub fn label_index(&self, label: &str) -> Option<usize> {
        self.events.iter().position(|e| e.label.eq_ignore_ascii_case(label))
    }
}

#[derive(Debug, Default)]
pub struct ConDb {
    pub cons: Vec<Conversation>,
    /// Conversations listed for each mission (ConList_MissionNN).
    pub missions: HashMap<i32, Vec<usize>>,
}

impl ConDb {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.cons.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The conversations a mission's map can use, owned by `owner` (an actor's BindName).
    pub fn for_owner<'a>(&'a self, mission: i32, owner: &'a str) -> impl Iterator<Item = usize> + 'a {
        self.missions
            .get(&mission)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |i| self.cons[*i].owner_name.eq_ignore_ascii_case(owner))
    }
}

struct Obj {
    class: String,
    props: Vec<Property>,
    rest: Vec<u8>,
}

fn p<'a>(o: &'a Obj, name: &str) -> Option<&'a Value> {
    o.props.iter().find(|x| x.name.eq_ignore_ascii_case(name) && x.index == 0).map(|x| &x.value)
}
fn s(o: &Obj, name: &str) -> String {
    match p(o, name) {
        Some(Value::Str(v)) => v.clone(),
        Some(Value::Name(v)) => v.clone(),
        _ => String::new(),
    }
}
fn b(o: &Obj, name: &str) -> bool {
    matches!(p(o, name), Some(Value::Bool(true)))
}
fn i(o: &Obj, name: &str) -> i32 {
    match p(o, name) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        _ => 0,
    }
}
fn fl(o: &Obj, name: &str) -> f32 {
    match p(o, name) {
        Some(Value::Float(v)) => *v,
        _ => 0.0,
    }
}
fn byte(o: &Obj, name: &str) -> u8 {
    match p(o, name) {
        Some(Value::Byte(v)) => *v,
        _ => 0,
    }
}
fn obj(o: &Obj, name: &str) -> i32 {
    match p(o, name) {
        Some(Value::Object(v)) => *v,
        _ => 0,
    }
}

fn read_obj(pkg: &Package, idx: usize) -> Option<Obj> {
    let e = pkg.exports.get(idx)?;
    let data = pkg.export_data(e);
    let mut r = Reader::new(data, 0);
    if e.flags & RF_HAS_STACK != 0 {
        skip_state_frame(&mut r).ok()?;
    }
    let props = read_properties(pkg, &mut r).ok()?;
    let rest = data[r.pos().min(data.len())..].to_vec();
    Some(Obj { class: pkg.class_name(e.class_index).to_string(), props, rest })
}

/// A class reference stored as an object (Class<Inventory>): its name.
fn class_ref(pkg: &Package, o: &Obj, name: &str) -> Option<String> {
    let r = obj(o, name);
    (r != 0).then(|| pkg.object_name(r))
}

impl ConDb {
    /// Load DeusExConText.u from the game's System folder.
    pub fn load(system_dir: &Path) -> Option<ConDb> {
        let path = unreal_pkg::paths::find_ci(system_dir, "DeusExConText.u")?;
        let pkg = Package::open(path).ok()?;
        Some(Self::from_package(&pkg))
    }

    pub fn from_package(pkg: &Package) -> ConDb {
        let objs: Vec<Option<Obj>> = (0..pkg.exports.len()).map(|k| read_obj(pkg, k)).collect();
        let get = |r: i32| -> Option<&Obj> { if r > 0 { objs.get(r as usize - 1)?.as_ref() } else { None } };
        let flag_list = |mut r: i32| -> Vec<FlagRef> {
            let mut out = Vec::new();
            let mut guard = 0;
            while let Some(f) = get(r) {
                out.push(FlagRef { name: s(f, "flagName"), value: b(f, "value"), expiration: i(f, "expiration") });
                r = obj(f, "nextFlagRef");
                guard += 1;
                if guard > 1000 {
                    break;
                }
            }
            out
        };
        // Conversation export index (1-based) -> our index.
        let mut con_index: HashMap<i32, usize> = HashMap::new();
        let mut con_exports = Vec::new();
        for (k, o) in objs.iter().enumerate() {
            if let Some(o) = o {
                if o.class == "Conversation" {
                    con_index.insert(k as i32 + 1, con_exports.len());
                    con_exports.push(k as i32 + 1);
                }
            }
        }
        let mut cons = Vec::new();
        for &ce in &con_exports {
            let c = get(ce).unwrap();
            let mut events = Vec::new();
            let mut r = obj(c, "eventList");
            let mut guard = 0;
            while let Some(e) = get(r) {
                let label = s(e, "label");
                let kind = match e.class.as_str() {
                    "ConEventSpeech" => {
                        let sp = get(obj(e, "conSpeech"));
                        EventKind::Speech {
                            speaker: s(e, "speakerName"),
                            speaking_to: s(e, "speakingToName"),
                            text: sp.map(|x| s(x, "speech")).unwrap_or_default(),
                            sound_id: sp.map_or(-1, |x| match p(x, "soundID") {
                                Some(Value::Int(v)) => *v,
                                _ => 0,
                            }),
                            continued: b(e, "bContinued"),
                            bold: b(e, "bBold"),
                            computer_font: byte(e, "speechFont") == 1,
                        }
                    }
                    "ConEventChoice" => {
                        let mut choices = Vec::new();
                        let mut cr = obj(e, "ChoiceList");
                        let mut g2 = 0;
                        while let Some(ch) = get(cr) {
                            let skill = class_ref(pkg, ch, "skillNeeded").map(|c| (c, i(ch, "skillLevelNeeded")));
                            choices.push(Choice {
                                text: s(ch, "choiceText"),
                                label: s(ch, "choiceLabel"),
                                display_as_speech: b(ch, "bDisplayAsSpeech"),
                                sound_id: i(ch, "soundID"),
                                skill,
                                flags: flag_list(obj(ch, "flagRef")),
                            });
                            cr = obj(ch, "nextChoice");
                            g2 += 1;
                            if g2 > 100 {
                                break;
                            }
                        }
                        EventKind::Choice { clear_screen: b(e, "bClearScreen"), choices }
                    }
                    "ConEventSetFlag" => EventKind::SetFlag(flag_list(obj(e, "flagRef"))),
                    "ConEventCheckFlag" => EventKind::CheckFlag { flags: flag_list(obj(e, "flagRef")), label: s(e, "setLabel") },
                    "ConEventCheckObject" => EventKind::CheckObject {
                        object_name: s(e, "objectName"),
                        class: class_ref(pkg, e, "checkObject"),
                        fail_label: s(e, "failLabel"),
                    },
                    "ConEventTransferObject" => EventKind::TransferObject {
                        object_name: s(e, "objectName"),
                        class: class_ref(pkg, e, "giveObject"),
                        count: i(e, "transferCount"),
                        from: s(e, "fromName"),
                        to: s(e, "toName"),
                        fail_label: s(e, "failLabel"),
                    },
                    "ConEventMoveCamera" => EventKind::MoveCamera {
                        camera_type: byte(e, "cameraType"),
                        position: byte(e, "cameraPosition"),
                        transition: byte(e, "cameraTransition"),
                        actor: s(e, "cameraActorName"),
                        offset: match p(e, "cameraOffset") {
                            Some(Value::Vector(v)) => *v,
                            _ => [0.0; 3],
                        },
                        rotation: match p(e, "rotation") {
                            Some(Value::Rotator(r)) => *r,
                            _ => [0; 3],
                        },
                        height: fl(e, "heightModifier"),
                        center: fl(e, "centerModifier"),
                        distance: fl(e, "distanceMultiplier"),
                    },
                    "ConEventAnimation" => EventKind::Animation {
                        owner: s(e, "eventOwnerName"),
                        sequence: s(e, "sequence"),
                        finish: b(e, "bFinishAnim"),
                        looped: b(e, "bLoopAnim"),
                    },
                    "ConEventTrade" => EventKind::Trade { owner: s(e, "eventOwnerName") },
                    "ConEventJump" => EventKind::Jump {
                        label: s(e, "jumpLabel"),
                        con: con_index.get(&obj(e, "jumpCon")).copied(),
                    },
                    "ConEventRandomLabel" => {
                        // Native DynamicArray<String>: compact count, then the strings.
                        let mut rr = Reader::new(&e.rest, 0);
                        let n = rr.compact_index().unwrap_or(0).max(0);
                        let labels = (0..n).filter_map(|_| rr.fstring().ok()).collect();
                        EventKind::Random {
                            labels,
                            cycle: b(e, "bCycleEvents"),
                            cycle_once: b(e, "bCycleOnce"),
                            cycle_random: b(e, "bCycleRandom"),
                        }
                    }
                    "ConEventTrigger" => EventKind::Trigger { tag: s(e, "triggerTag") },
                    "ConEventAddGoal" => EventKind::AddGoal {
                        name: s(e, "goalName"),
                        completed: b(e, "bGoalCompleted"),
                        text: s(e, "goalText"),
                        primary: b(e, "bPrimaryGoal"),
                    },
                    "ConEventAddNote" => EventKind::AddNote { text: s(e, "noteText") },
                    "ConEventAddSkillPoints" => EventKind::AddSkillPoints { points: i(e, "pointsToAdd"), message: s(e, "awardMessage") },
                    "ConEventAddCredits" => EventKind::AddCredits(i(e, "creditsToAdd")),
                    "ConEventCheckPersona" => EventKind::CheckPersona {
                        persona: byte(e, "personaType"),
                        condition: byte(e, "condition"),
                        value: i(e, "value"),
                        label: s(e, "jumpLabel"),
                    },
                    "ConEventComment" => EventKind::Comment(s(e, "commentText")),
                    _ => EventKind::End,
                };
                events.push(Event { label, kind });
                r = obj(e, "nextEvent");
                guard += 1;
                if guard > 5000 {
                    break;
                }
            }
            cons.push(Conversation {
                name: s(c, "conName"),
                description: s(c, "Description"),
                owner_name: s(c, "conOwnerName"),
                datalink: b(c, "bDataLinkCon"),
                display_once: b(c, "bDisplayOnce"),
                first_person: b(c, "bFirstPerson"),
                non_interactive: b(c, "bNonInteractive"),
                random_camera: b(c, "bRandomCamera"),
                can_be_interrupted: b(c, "bCanBeInterrupted"),
                cannot_be_interrupted: b(c, "bCannotBeInterrupted"),
                invoke_bump: b(c, "bInvokeBump"),
                invoke_frob: b(c, "bInvokeFrob"),
                invoke_sight: b(c, "bInvokeSight"),
                invoke_radius: b(c, "bInvokeRadius"),
                radius: i(c, "radiusDistance"),
                events,
                flags: flag_list(obj(c, "flagRefList")),
                id: i(c, "conversationID"),
                audio_package: s(c, "audioPackageName"),
            });
        }
        let mut missions: HashMap<i32, Vec<usize>> = HashMap::new();
        for o in objs.iter().flatten() {
            if o.class != "ConversationList" {
                continue;
            }
            let m = i(o, "missionNumber");
            let mut r = obj(o, "conversations");
            let mut guard = 0;
            let list = missions.entry(m).or_default();
            while let Some(item) = get(r) {
                if let Some(ci) = con_index.get(&obj(item, "conObject")) {
                    list.push(*ci);
                }
                r = obj(item, "next");
                guard += 1;
                if guard > 10000 {
                    break;
                }
            }
        }
        ConDb { cons, missions }
    }
}

/// Speech audio of one package (DeusExConAudio<name>.u): soundID -> Sound object name.
pub struct ConAudio {
    pub package: Package,
    pub sounds: Vec<i32>,
}

impl ConAudio {
    pub fn load(system_dir: &Path, audio_package: &str) -> Option<ConAudio> {
        let want = format!("DeusExConAudio{audio_package}.u");
        let path = unreal_pkg::paths::find_ci(system_dir, &want)?;
        let package = Package::open(path).ok()?;
        let k = package.exports.iter().position(|e| package.class_name(e.class_index) == "ConAudioList")?;
        let o = read_obj(&package, k)?;
        let mut r = Reader::new(&o.rest, 0);
        let n = r.compact_index().ok()?.max(0);
        let sounds = (0..n).map(|_| r.compact_index().unwrap_or(0)).collect();
        Some(ConAudio { package, sounds })
    }

    /// The Sound export for a soundID.
    pub fn export(&self, sound_id: i32) -> Option<&unreal_pkg::Export> {
        let r = *self.sounds.get(usize::try_from(sound_id).ok()?)?;
        (r > 0).then(|| &self.package.exports[r as usize - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CON_GAME=<game root> cargo test -p dx-level conversations
    #[test]
    fn island_conversations_decode() {
        let Ok(root) = std::env::var("CON_GAME") else { return };
        let sys = Path::new(&root).join("System");
        let db = ConDb::load(&sys).expect("DeusExConText.u");
        assert!(db.cons.len() > 1900);
        let m1 = &db.missions[&1];
        assert!(!m1.is_empty());
        let mut owners: Vec<&str> = m1.iter().map(|i| db.cons[*i].owner_name.as_str()).collect();
        owners.sort();
        owners.dedup();
        println!("mission 1: {} conversations, owners {:?}", m1.len(), owners);
        let paul = db.cons.iter().find(|c| c.name.contains("PaulAtDock") || c.name.contains("MeetPaul")).or_else(|| db.cons.iter().find(|c| c.owner_name == "PaulDenton"));
        if let Some(c) = paul {
            println!("{} owner {} fp {} events {}", c.name, c.owner_name, c.first_person, c.events.len());
            for e in c.events.iter().take(12) {
                println!("  [{}] {:?}", e.label, e.kind);
            }
        }
        let randoms = db.cons.iter().flat_map(|c| &c.events).filter(|e| matches!(&e.kind, EventKind::Random { labels, .. } if !labels.is_empty())).count();
        assert!(randoms > 500, "random labels decoded: {randoms}");
        let audio = ConAudio::load(&sys, "Mission01").expect("audio");
        println!("audio list {} entries; 314 -> {:?}", audio.sounds.len(), audio.export(314).map(|e| &e.name));
        assert!(audio.export(314).is_some());
    }
}

