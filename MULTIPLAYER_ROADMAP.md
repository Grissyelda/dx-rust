# DX RUST multiplayer roadmap

Multiplayer has two families of competitive play, plus co-op:

- **Classic** rebuilds the original game's multiplayer patch (v1112fm) from its own scripts, with no mods and none
  of our rebalancing.
- **Modern** is a new immersive-sim player-versus-player game. The level itself is a third side, and every Modern
  enhancement (live-lens scope, terminals used in the world, ragdolls, Smarter AI) comes along.
- **Co-op** stays as it is: up to 16 players through the story in one shared world.

The general project roadmap is [ROADMAP.md](ROADMAP.md).

## Where things stand

| Phase | What | Status |
|---|---|---|
| 0 | Groundwork | Done |
| 1 | Classic deathmatch | Built; needs playtests against the original |
| 2 | Modern foundations | Not started |
| 3 | Modern mechanics | Not started |
| 4 | Modern modes | Not started |
| 5 | Maps | Not started |
| 6 | Bots, balance and release | Not started |

Classic needs only the groundwork, so it ships first. Modern's maps are built alongside its modes, and both tracks
meet in the release phase.

## The four ideas behind Modern

- **The building is a third side.** Neutral guards, cameras, turrets, alarms and locked doors react to everyone,
  and any player can turn them against the others.
- **Information is a weapon.** Camera feeds, radar pings, keypad codes, emails and interrogations decide fights
  before a shot is fired.
- **Non-lethal matters.** Knocked-out players can be interrogated, carried, hidden, revived or finished off.
- **Energy is scarce.** Strong augmentations drain fast and bioelectric cells are rare.

## Design decisions

These are the working assumptions. Each can change after playtests.

| Question | Decision | Why |
|---|---|---|
| How much power does progression give? | Unlocks add options; a fixed loadout budget per match keeps everyone equal. Hosts can choose *Full* for private rooms. | Returning players get variety; newcomers can still win. |
| Where does progression live? | On each PC first; signed by the lobby server for public rooms later. | No accounts needed to start. |
| What do you lose in Extraction? | What you picked up that match, never your loadout. | Tension without punishing losses. |
| Is the level always alive? | Always in Modern objective modes; *Off / Light / Full* in Deathmatch and Team Deathmatch. | Deathmatch needs a pure option. |
| Third person in PvP? | A host option, off by default. | Peeking round corners is unfair to first-person players. |
| Team size | Built for up to 8 v 8, tuned for 4 v 4. | The single-player maps are dense. |
| Mods in Classic? | None at all. Our rebalanced pickup layouts move to Modern. | The original patch had none. |

## Phase 0: groundwork (done)

Turns the single deathmatch we had into a framework both families plug into. Already there before this phase: co-op
for 16, the lobby server with room codes and a browser, direct codes, LAN discovery, UPnP, ready-up, chat, mod
parity checks, deathmatch and team deathmatch on the five original maps, deathmatch characters with weapon-skin
unlocks, the scoreboard and match settings.

- [x] **Mode framework.** One `MatchMode` trait (`matchmode.rs`) covers rules, scoring, spawns, pickups, damage and
  rewards, so each mode is a module rather than a set of branches.
- [x] **Family switch in the Host Room.** Classic or Modern is picked first. It sets the look (Classic switches
  mods off) and decides which game modes are offered.
- [x] **Map data format.** Per map and per mode, in `data/deathmatch.ron`: spawn sets, play-area blockers,
  objective spots, loot tables and codes made fresh each match. A test checks the data against the map files.
- [x] **Operative profile v1.** Deathmatch characters keep credits (from kills, heals and matches), weapon mastery,
  unlocks and saved loadouts, on each PC for now.
- [x] **Network budget.** `DXVIEW_NETBUDGET=1` prints the bytes sent per message kind every 5 seconds; large
  updates are split into smaller messages.
- [x] **Kill credit for grenades, mines and turrets.** Blast and turret hits carry the player who threw, planted or
  owns them.

## Phase 1: Classic deathmatch (built)

Classic is the original multiplayer patch ported from its own scripts (`DeusExMPGame`, `DeathMatchGame`,
`TeamDMGame`, `BasicTeamDMGame`, `AdvTeamDMGame`). Where descriptions online differ from the code, the code wins.
The multiplayer values are read at start-up from your own copy of the game; nothing from it is copied into this
repository.

| Game type | Starting skill points | Per kill | Starting augmentations | Augmentations per kill | Skills start at |
|---|---|---|---|---|---|
| Deathmatch | host sets | host sets | host sets | host sets | host sets |
| Basic Team DM | 0 | 0 | 9 | 0 | Advanced |
| Advanced Team DM | 2,000 | 2,000 | 2 | 2 | Trained |
| Custom Team DM | host sets | host sets | host sets | host sets | host sets |

- [x] **Game types and host settings** as the original defines them: skill points, augmentations, starting skill
  level, kill or time limit, friendly fire.
- [x] **Teams.** UNATCO troopers against NSF terrorists in the original team outfits. In free-for-all Deathmatch,
  pick JC, an NSF terrorist, a UNATCO trooper or an MJ12 agent in *Player Setup*.
- [x] **The original skill-buying screen** (number keys) and augmentations handed out in preference order on each
  kill.
- [x] **Original pickups.** Each map's own items; weapons stay where they lie for everyone, other items come back
  after their respawn time.
- [x] **Original combat numbers.** Every weapon, skill and augmentation's multiplayer values; damage by head, body
  and legs; the sniper rifle hits hard only through its scope; six seconds of protection after spawning.
- [x] **Multiplayer security.** Turrets and cameras stay off until someone switches them on at a security computer;
  then they protect that player (and his team) and can't be destroyed.
- [x] **Original screens and messages.** Kill lines, the death screen with the killer's details, the centred
  notices, the scoreboard and the end-of-match screen.
- [ ] **Gate:** a full match of each game type plays like the original, side by side with it.

Still to do in Classic: cameras watching every player (they only watch the host's own player now), choosing your
team, the augmentation-preference screen, and showing your own class model in mirrors and first person.

## Phase 2: Modern foundations

Delivers Modern Deathmatch and Team Deathmatch with a living level and progression between matches. Every later
mode builds on it.

### The living level
- [ ] **Neutral security.** Guards, cameras and turrets belong to the level, neutral to all players until provoked
  (unsilenced fire, being seen in restricted rooms, attacking staff, a camera spotting a drawn weapon).
- [ ] **Heat per player.** Each player has their own alert level; guards hunt whoever caused it, not everyone.
- [ ] **Hacked security changes hands.** A hacked turret or camera works for the hacker (and their team) for a
  while; another player can hack it back.
- [ ] **Level activity setting**: Off / Light (cameras and alarms only) / Full, set by the host.

### Progression between matches
- [ ] **Credits** for objectives, non-lethal takedowns, hacks, assists and kills, with a breakdown after each match.
- [ ] **Loadout budget.** Each loadout spends a fixed number of points on skills, augmentations and perks; unlocks
  widen the choice, never the budget. Host option: *Equal* (default) or *Full*.
- [ ] **Six class templates** to start from and edit:

| Class | Plays as | Signature perk |
|---|---|---|
| Infiltrator | cloak, silent movement, lockpicks | uniforms last longer |
| Netrunner | hacking, turret and camera control | remote hacking range |
| Enforcer | heavy weapons, ballistic protection | melee bash |
| Medic | revives, healing others | extra medkits |
| Saboteur | mines, EMP, explosives | traps on terminals |
| Marksman | rifles, steady scope | spotting marks rivals for the team |

- [ ] **Weapon mastery.** Kills and hits with a weapon unlock its original mods (accuracy, clip, range, recoil,
  reload, scope, silencer, laser) and its skins; fitting a mod costs loadout points.
- [ ] **Mods shown on the weapon**: scope, laser and silencer visible in first and third person (needed before
  mastery ships).
- [ ] **Cosmetics**: outfits, the existing kill-earned skins, name-tag colours, calling cards.
- [ ] **Loadout screen** in the Multiplayer menu: class, skills, augmentations, perks, weapons, mods and outfit,
  with the budget shown.

### Modern combat
- [ ] Every Modern enhancement works in PvP: live-lens scope, aim-down-sights and sway, mantling, smooth lean,
  cover, ragdolls, fluid blood, fire and water interactions, real-time shadows.
- [ ] **Light and sound as cover.** Players see each other by the same light levels guards use; footsteps by
  surface, thrown objects and unsilenced shots carry, with an optional sound radar.
- [ ] **Gate:** an 8-player Modern Team Deathmatch with the level on *Full* holds 60 fps on a mid-range PC and stays
  within the network budget.

## Phase 3: Modern mechanics

What no other shooter has. Each is its own module with a host switch, built in this order because each leans on the
one before.

- [ ] **Downed state and revive.** Non-lethal takedowns (prod, baton, tranquilizer darts, gas) leave a player down
  for a while instead of dead. Teammates revive with a medkit; rivals can finish, carry or hide the body.
- [ ] **Interrogation.** A multitool on a downed rival for about four seconds reveals rival positions, his objective
  progress, or one killphrase fragment.
- [ ] **Uniforms.** Wear a knocked-out guard's uniform and the level treats you as staff. Running, a drawn weapon
  near guards or a restricted room gives you away (guards grow suspicious first). Rival players see a guard until
  they look closely or use the Vision augmentation.
- [ ] **Overwatch seat.** The security room's computer shows every camera live and can lock doors, flip turrets and
  send guards after a rival. The seated player's body stays in the world, exposed.
- [ ] **Sector lockdowns.** Terminals seal blast doors and switch on the level's hazards (gas, steam, electrified
  water, fans) for a while, with a cooldown.
- [ ] **Framing.** Guards who find a body or a dropped weapon blame its owner: plant a rival's gun beside your
  victim and the level hunts him.
- [ ] **Killphrases.** Each player gets a three-word phrase per match. Fragments hide in datacubes, emails and
  interrogations; saying all three to a rival's face kills him at once. Works as an option in any Modern mode and
  as its own mode.
- [ ] **Counter table.** Every power has an answer (cloak against Vision and thermal, radar transparency against
  eyes, hacked turrets against EMP, uniforms against a close look, Overwatch against cutting the room's power),
  kept in data and tested in playtests.

## Phase 4: Modern modes

Simplest rules first; each mode reuses Phases 2 and 3 and adds only its own objective.

| Order | Mode | Players | Objective | What makes it Deus Ex |
|---|---|---|---|---|
| 1 | Deathmatch / Team Deathmatch | free-for-all or 2 teams | most kills | the level as a third side |
| 2 | Data Heist (capture the flag) | 2 teams | carry the rival's data core home | the core is a heavy object |
| 3 | Uplink (domination) | 2 teams | hold hacked terminals | each terminal controls its sector |
| 4 | Extraction | free-for-all or squads of 3 | get canisters out alive | vaults, guards, interrogation |
| 5 | Infiltration | attackers vs defenders | steal one item / stop it | defenders get the Overwatch seat |
| 6 | Juggernaut | 1 vs all | kill the juggernaut | Gunther-style armour and a heavy gun |

**Data Heist**
- [ ] Each team's data core sits in a vault behind its own team's security.
- [ ] The core is carried like a heavy object: no weapons while carrying, slower, can be thrown to a teammate,
  hidden in a container or dropped in water (it floats).
- [ ] It pings its position to the other team every 20 seconds, unless carried through shielded rooms or with
  Radar Transparency running.

**Uplink**
- [ ] Control points are terminals you hack (faster with the Computer skill); rivals hack them back.
- [ ] Holding a point gives your team that sector's cameras, turrets, doors and lights.
- [ ] Score ticks for each point held; mines and turrets make defending worth it.

**Extraction**
- [ ] Data and augmentation canisters sit in locked vaults and guarded offices: go in quietly (lockpicks, codes,
  uniforms) or loudly (explosives that draw guards and players).
- [ ] Extraction points open in stages and are announced; the player extracting gives off a signal.
- [ ] A knocked-out extractor can be interrogated for his canisters; what he picked up that match drops when he
  dies.
- [ ] Extracted canisters pay credits and progression rewards.

**Infiltration and Juggernaut**
- [ ] Infiltration: rounds with sides swapped; defenders start with the Overwatch seat; attackers win by stealing
  one item or cutting the security room's power.
- [ ] Juggernaut: high armour, a heavy weapon, pinged every 15 seconds; whoever kills him becomes the next one.
- [ ] Killphrase as a short standalone mode.

## Phase 5: maps

Modern reuses the single-player maps, which already have guards, cameras, terminals, keypads, vents and hazards.
Each needs only data: a closed-off play area, spawn sets, objective spots, and loot and code tables randomised per
match.

| Map | Modes | Why it fits |
|---|---|---|
| Liberty Island (`01_NYC_UNATCOIsland`) | Data Heist, Team DM | the statue and UNATCO's side are two natural bases |
| Battery Park (`02_NYC_BatteryPark`) | Data Heist, Uplink | Castle Clinton against the subway |
| UNATCO HQ (`01_NYC_UNATCOHQ`) | Extraction, Infiltration | offices, the security room, Manderley's office as the vault |
| NSF Headquarters (`04_NYC_NSFHQ`) | Extraction, Infiltration | rooftops, a guarded transmitter, vents |
| VersaLife (`06_HongKong_VersaLife`) | Extraction | offices and labs behind keycards |
| Wan Chai Market (`06_HongKong_WanChai_Market`) | Uplink, Deathmatch | dense stalls and alleys, several sectors |
| Vandenberg Command (`12_Vandenberg_Cmd`) | Uplink, Infiltration | a command centre with turrets and bots |
| Paris Catacombs (`10_Paris_Catacombs`) | Extraction, Deathmatch | dark tunnels where light and sound decide fights |
| 'Ton Hotel (`04_NYC_Hotel`) | Deathmatch | small, fast, two floors |
| The five original multiplayer maps (`DXMP_*`) | all Classic modes; Modern Deathmatch / Team DM | made for multiplayer |

- [ ] **Play-area tool**: an in-game editor (debug menu) to place blockers, spawns and objective spots and write the
  map's data file.
- [ ] **Story clean-up per map**: mission scripts, conversations and level exits off in multiplayer; guards keep
  their patrols.
- [ ] **Randomised secrets**: keypad codes, logins and vault contents change every match, so knowledge has to be
  found, not memorised.
- [ ] **First three maps**: Liberty Island (Data Heist), UNATCO HQ (Extraction), Wan Chai Market (Uplink).
- [ ] **Later**: custom multiplayer maps.

## Phase 6: bots, balance and release

Makes matches possible with few players and safe to release publicly.

- [ ] **Bot players.** The pawn AI drives player bodies that fill empty slots, pick a class, hack, take cover and go
  for objectives; difficulty set by the host.
- [ ] **Balance pass.** Time-to-kill targets, augmentation energy costs, loadout prices and the counter table, all
  in data files so they can be tuned without a rebuild.
- [ ] **Trusted progression.** Public rooms accept profiles signed by the lobby server; private rooms take any.
- [ ] **Playtests** over the internet of each mode at 4 v 4 and 8 v 8, Steam Deck and Android included (Android
  joins Classic rooms only).
- [ ] **Help screens**: a short how-to per mode and per mechanic, in the menu style.
- [ ] **Release** as part of v2.0, with the lobby browser showing each room's family and mode.

## Open questions

- Should bots also be offered in Classic, or Modern only?
- Which mechanics are too far from Deus Ex to be on by default: uniforms, killphrases, Overwatch, framing,
  interrogation?
- Does Classic need the original's dedicated-server option, or is a player hosting enough?
- Should Modern progression also reward co-op (credits for missions finished together)?
