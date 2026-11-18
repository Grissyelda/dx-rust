//! Readable things (InformationDevices: datacubes, books, newspapers). Frobbing one runs its text (textTag in its
//! TextPackage) through the DeusExTextParser the way InformationDevices.ProcessTag does - every <P> a new text
//! window, <JC> / <JR> / <JL> aligning the current one, <DC=r,g,b> its colour - and shows it in the HUD's
//! HUDInformationDisplay (drawn by classic_hud.rs with the game's own bitmap font). The window closes when the
//! player looks away (FrobTarget is no longer the device), frobs it again, or it's destroyed.

use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;

/// One TextWindow of the information display: its text, alignment (0 left, 1 centre, 2 right) and colour.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Para {
    pub text: String,
    pub align: u8,
    pub color: [u8; 3],
    /// The Nameless Mod's <B>: the window's font is its boldFont (FontMenuHeaders).
    pub bold: bool,
}

/// HUDInformationDisplay.colInfoText.
pub const INFO_TEXT: [u8; 3] = [250, 250, 250];

/// What InformationDevices.CreateInfoWindow made of a text: the windows and the DataVault string.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub paras: Vec<Para>,
    pub vault: String,
}

/// DeusExTextParser + InformationDevices.ProcessTag. Whitespace before each token is eaten; text runs to the next
/// tag; tags the device doesn't handle (<B>, <I>, <U>, <FILE>, <EMAIL>) are dropped; comments are skipped.
pub fn parse(raw: &str) -> Parsed {
    let player = crate::travel::player_name();
    parse_with(raw, &player, false)
}

/// The Nameless Mod's TNMInformationDevices.ProcessTag: as the original, plus <B> (the next text makes the window
/// bold) and the %LT% / %GT% escapes for < and >.
pub fn parse_tnm(raw: &str) -> Parsed {
    let player = crate::travel::player_name();
    parse_with(raw, &player, true)
}

fn parse_with(raw: &str, player: &str, tnm: bool) -> Parsed {
    let s = raw.replace('\0', "");
    let mut out = Parsed::default();
    // winText: the window being written to (None until the first token), and bSetText / bFirstParagraph.
    let mut cur: Option<Para> = None;
    let mut implicit = false;
    let mut set_text = false;
    let mut first_para = true;
    let mut bold = false;
    let new_para = || Para { text: String::new(), align: 0, color: INFO_TEXT, bold: false };
    let esc = |t: String| if tnm { t.replace("%LT%", "<").replace("%GT%", ">") } else { t };
    let mut rest = s.as_str();
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
        if rest.is_empty() {
            break;
        }
        if cur.is_none() {
            cur = Some(new_para());
            implicit = true;
            set_text = true;
        }
        // Text up to the next tag (or the end).
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = esc(rest[..end].trim_end_matches(['\r', '\n']).replace(['\r', '\n'], " "));
            rest = &rest[end..];
            add_text(&mut cur, &mut set_text, &mut out.vault, &text);
            if bold {
                cur.as_mut().unwrap().bold = true;
                bold = false;
            }
            continue;
        }
        let Some(gt) = rest.find('>') else {
            // A parse error: the rest is shown as text.
            let text = rest.to_string();
            rest = "";
            add_text(&mut cur, &mut set_text, &mut out.vault, &text);
            continue;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let upper = tag.trim().to_ascii_uppercase();
        match upper.as_str() {
            "P" => {
                // TT_NewParagraph: a new text window (the implicit first one goes if nothing was written to it).
                if let Some(p) = cur.take() {
                    if !(implicit && p.text.is_empty()) {
                        out.paras.push(p);
                    }
                }
                cur = Some(new_para());
                implicit = false;
                if !first_para {
                    out.vault.push('\n');
                }
                first_para = false;
                set_text = true;
            }
            "B" if tnm => bold = true,
            "JC" => cur.as_mut().unwrap().align = 1,
            "JR" => cur.as_mut().unwrap().align = 2,
            "JL" => cur.as_mut().unwrap().align = 0,
            "PLAYERNAME" => add_text(&mut cur, &mut set_text, &mut out.vault, player),
            "PLAYERFIRSTNAME" => {
                add_text(&mut cur, &mut set_text, &mut out.vault, player.split(' ').next().unwrap_or(""))
            }
            "COMMENT" => match rest.to_ascii_uppercase().find("</COMMENT>") {
                Some(e) => rest = &rest[e + "</COMMENT>".len()..],
                None => rest = "",
            },
            _ => {
                // <DC=r,g,b> (TT_DefaultColor) and <TC=r,g,b> (TT_TextColor): SetTextColor on the current window.
                if let Some(v) = upper.strip_prefix("DC=").or_else(|| upper.strip_prefix("TC=")) {
                    let c: Vec<u8> = v.split(',').filter_map(|n| n.trim().parse::<u32>().ok()).map(|n| n.min(255) as u8).collect();
                    if c.len() == 3 {
                        cur.as_mut().unwrap().color = [c[0], c[1], c[2]];
                    }
                }
            }
        }
    }
    if let Some(p) = cur.take() {
        if !(implicit && p.text.is_empty()) {
            out.paras.push(p);
        }
    }
    out
}

fn add_text(cur: &mut Option<Para>, set_text: &mut bool, vault: &mut String, text: &str) {
    let p = cur.as_mut().unwrap();
    if *set_text {
        p.text = text.to_string();
    } else {
        p.text.push_str(text);
    }
    vault.push_str(text);
    *set_text = false;
}

#[derive(Clone, Debug)]
pub struct Open {
    pub paras: Vec<Para>,
    /// The device being read (its blocker): the window closes when it's no longer the frob target.
    pub blocker: usize,
    /// The Nameless Mod's page hints under the window (multi-page books).
    pub footer: Option<Footer>,
}

/// TNMInformationDevices' page hints, drawn under the information window in white: the "previous" hint on the left
/// when there is a page before this one, the "next" hint on the right when there is one after, and the page count
/// between them (all three texts come from the mod's class defaults).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Footer {
    pub prev: Option<String>,
    pub next: Option<String>,
    pub page: Option<String>,
}

#[derive(Resource, Default)]
pub struct InfoDisplay {
    pub open: Option<Open>,
    pkgs: HashMap<String, Option<unreal_pkg::Package>>,
    /// TNMInformationDevices.currentpage per device (its blocker); starts at -1 like the class default.
    pages: HashMap<usize, i32>,
}

/// A Nameless Mod readable (TNMInformationDevices): its pages, the DataVault note it adds and its labels.
pub struct TnmReadable<'a> {
    pub package: &'a str,
    /// textTag[0..]: one text per page.
    pub tags: &'a [String],
    pub pages: i32,
    /// GetAltTag overrides (SourceCodeCube: "SourceCube_" + page), used instead of textTag.
    pub alt_prefix: Option<&'a str>,
    /// notetag: its text becomes a DataVault note the first time the device is read.
    pub note_tag: &'a str,
    /// nextpagetext, prevpagetext, totalpagetext (two %d: this page, the page count).
    pub page_texts: (&'a str, &'a str, &'a str),
}

/// What a frob on a readable did.
pub enum Frobbed {
    /// The window opened; the DataVault string if bAddToVault (added once).
    Opened(Option<String>),
    /// It was open: closed.
    Closed,
    /// No text and no image: msgNoText.
    NoText,
}

impl InfoDisplay {
    /// InformationDevices.Frob: open the window for `tag` (in `package`), or close it if it's showing. `image` is the
    /// device's imageClass with its DataVault description: given to the player, and shown as "[Image: ...]" when
    /// there's no text.
    pub fn frob(
        &mut self,
        system: &std::path::Path,
        package: &str,
        tag: &str,
        blocker: usize,
        at: DVec3,
        image: Option<(&str, &str)>,
        add_to_vault: bool,
        labels: (&str, &str),
    ) -> Frobbed {
        if self.open.as_ref().is_some_and(|o| o.blocker == blocker) {
            self.open = None;
            return Frobbed::Closed;
        }
        let pkg = if package.is_empty() { "DeusExText" } else { package };
        let p = self
            .pkgs
            .entry(pkg.to_ascii_lowercase())
            .or_insert_with(|| unreal_pkg::Package::open(system.join(format!("{pkg}.u"))).ok());
        let mut parsed = None;
        if !tag.is_empty() {
            // Lay D Denton's InformationDevices: her FemaleTextTag ("FemJC" + textTag) first, else the original.
            let fem = crate::travel::female()
                .then(|| p.as_ref().and_then(|p| p.text_named(&format!("FemJC{tag}"))))
                .flatten();
            if let Some(raw) = fem.or_else(|| p.as_ref().and_then(|p| p.text_named(tag))) {
                parsed = Some(parse(&raw));
            }
        }
        let mut vault = None;
        let mut paras = None;
        if let Some(pr) = parsed {
            if add_to_vault {
                vault = Some(pr.vault.clone());
            }
            paras = Some(pr.paras);
        }
        if let Some((class, desc)) = image {
            // DataVaultImage given to the reader: "Image %s added to DataVault" when it's new; the window shows
            // "[Image: %s]" only when there was no text.
            if crate::consys::add_image(class) {
                crate::classic_hud::client_message(&labels.1.replacen("%s", desc, 1));
            }
            if paras.is_none() {
                paras = Some(vec![Para { text: labels.0.replacen("%s", desc, 1), align: 0, color: INFO_TEXT, bold: false }]);
            }
        }
        match paras {
            Some(paras) => {
                let _ = at;
                self.open = Some(Open { paras, blocker, footer: None });
                Frobbed::Opened(vault)
            }
            None => Frobbed::NoText,
        }
    }
}

impl InfoDisplay {
    fn text(&mut self, system: &std::path::Path, package: &str, tag: &str) -> Option<String> {
        if tag.is_empty() {
            return None;
        }
        let pkg = if package.is_empty() { "DeusExText" } else { package };
        let p = self
            .pkgs
            .entry(pkg.to_ascii_lowercase())
            .or_insert_with(|| unreal_pkg::Package::open(system.join(format!("{pkg}.u"))).ok());
        p.as_ref().and_then(|p| p.text_named(tag))
    }

    /// TNMInformationDevices.Frob: every frob turns to the next page (back to the first after the last) and shows
    /// it - the window never closes on a frob. The first read of a device with a notetag adds that text to the
    /// DataVault (returned in Opened). An imageClass is given as in InformationDevices.
    pub fn frob_tnm(
        &mut self,
        system: &std::path::Path,
        r: &TnmReadable,
        blocker: usize,
        image: Option<(&str, &str)>,
        labels: (&str, &str),
    ) -> Frobbed {
        let pages = r.pages.max(1);
        let page = {
            let p = self.pages.entry(blocker).or_insert(-1);
            *p += 1;
            if *p >= pages {
                *p = 0;
            }
            *p
        };
        let tag = match r.alt_prefix {
            // GetAltTag returns nothing past numberofpages + 1.
            Some(pre) if page <= pages + 1 => format!("{pre}{page}"),
            Some(_) => String::new(),
            None => r.tags.get(page as usize).cloned().unwrap_or_default(),
        };
        let mut paras = None;
        let mut note = None;
        if let Some(raw) = self.text(system, r.package, &tag) {
            paras = Some(parse_tnm(&raw).paras);
            if let Some(n) = self.text(system, r.package, r.note_tag) {
                let v = parse_tnm(&n).vault;
                if !crate::consys::has_vault_note(&v) {
                    note = Some(v);
                }
            }
        }
        if let Some((class, desc)) = image {
            if crate::consys::add_image(class) {
                crate::classic_hud::client_message(&labels.1.replacen("%s", desc, 1));
            }
            if paras.is_none() {
                paras = Some(vec![Para { text: labels.0.replacen("%s", desc, 1), align: 0, color: INFO_TEXT, bold: false }]);
            }
        }
        let Some(paras) = paras else {
            self.open = None;
            return Frobbed::NoText;
        };
        let footer = (pages > 1).then(|| Footer {
            prev: (page > 0).then(|| r.page_texts.1.to_string()),
            next: (page < pages - 1).then(|| r.page_texts.0.to_string()),
            page: Some(r.page_texts.2.replacen("%d", &(page + 1).to_string(), 1).replacen("%d", &pages.to_string(), 1)),
        });
        self.open = Some(Open { paras, blocker, footer });
        Frobbed::Opened(note)
    }
}

/// The page hints of the window on screen (classic_hud draws them under it).
pub fn footer() -> Option<Footer> {
    FOOT.lock().ok().and_then(|g| g.clone())
}

static FOOT: std::sync::Mutex<Option<Footer>> = std::sync::Mutex::new(None);

/// The text windows on screen (classic_hud draws them) and a key that changes with them.
static SHOWN: std::sync::Mutex<Option<(u64, Vec<Para>)>> = std::sync::Mutex::new(None);

pub fn shown() -> Option<(u64, Vec<Para>)> {
    SHOWN.lock().ok().and_then(|g| g.clone())
}

/// Text is being read: the crosshair and the frob display are hidden (InformationDevices.Frob).
pub fn reading() -> bool {
    SHOWN.lock().is_ok_and(|g| g.is_some())
}

pub fn reset_level() {
    if let Ok(mut g) = SHOWN.lock() {
        *g = None;
    }
    if let Ok(mut g) = FOOT.lock() {
        *g = None;
    }
}

/// InformationDevices.Tick: the window goes when the reader's FrobTarget isn't the device any more (or the device
/// is gone).
pub fn info_system(
    mut info: ResMut<InfoDisplay>,
    hl: Res<crate::frobui::FrobHighlight>,
    mut map: Local<u64>,
) {
    let _prof = crate::prof::scope("info::info_system");
    if crate::loader::new_map(&mut map) {
        // Devices are per map (blocker indices); a new map starts every book at its first page again.
        info.pages.clear();
        info.open = None;
    }
    if let Some(o) = &info.open {
        // (A broken device is no longer anyone's frob target either.)
        let looking = hl.target.as_ref().is_some_and(|t| t.blocker == Some(o.blocker));
        // (DXVIEW_INFO_HOLD keeps it up for test runs that frob a device without looking at it.)
        if !looking && !env_on!("DXVIEW_INFO_HOLD") {
            info.open = None;
        }
    }
    let want = info.open.as_ref().map(|o| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        o.paras.hash(&mut h);
        o.blocker.hash(&mut h);
        o.footer.hash(&mut h);
        h.finish()
    });
    if let Ok(mut g) = SHOWN.lock() {
        if g.as_ref().map(|s| s.0) != want {
            *g = info.open.as_ref().zip(want).map(|(o, k)| (k, o.paras.clone()));
            if let Ok(mut f) = FOOT.lock() {
                *f = info.open.as_ref().and_then(|o| o.footer.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datacube_paragraphs() {
        let raw = "<DC=255,255,255>\r\n<P>We'll make this one easy for you.  To open the door, use the code:\r\n<P>\r\n<P><JC><B>0012</B>\r\n<P>\r\n<P>Got it?\r\n<P>\r\n<P>Jaime\0";
        let p = parse_with(raw, "JC Denton", false);
        let texts: Vec<&str> = p.paras.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            ["We'll make this one easy for you.  To open the door, use the code:", "", "0012", "", "Got it?", "", "Jaime"]
        );
        assert_eq!(p.paras[2].align, 1);
        assert_eq!(p.paras[0].align, 0);
        assert_eq!(p.vault, "We'll make this one easy for you.  To open the door, use the code:\n\n0012\n\nGot it?\n\nJaime");
    }

    #[test]
    fn player_name_and_comment() {
        let p = parse_with("<P>Hello <PLAYERFIRSTNAME>.<COMMENT>hidden</COMMENT> Bye", "JC Denton", false);
        assert_eq!(p.paras.len(), 1);
        assert_eq!(p.paras[0].text, "Hello JC.Bye");
    }
}
